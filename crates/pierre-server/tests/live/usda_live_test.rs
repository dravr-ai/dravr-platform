// ABOUTME: Live nutrition and recipe tool tests against USDA FoodData Central
// ABOUTME: Built only with the live-e2e feature; fails, never skips, without a key or a reachable USDA
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Live USDA `FoodData` Central tests for `search_food`, `get_food_details`,
//! `analyze_meal_nutrition` and `validate_recipe`.
//!
//! Nothing here skips (carnet#805): a missing `USDA_API_KEY`, a timeout, a
//! rate limit, a 5xx or a degraded answer fails the test rather than passing
//! one that tested nothing.
//!
//! ```bash
//! USDA_API_KEY=... cargo test -p pierre_mcp_server --features live-e2e --test usda_live_test
//! ```
//!
//! The offline halves (validation, the no-key refusal) stay in
//! `nutrition_tools_integration_test` and `recipe_tools_integration_test`.

use std::env;
use std::time::Duration;

use anyhow::Result;
use pierre_config::environment::ServerConfig;
use pierre_core::models::{Tenant, User};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::json;
use tokio::time::timeout;
use uuid::Uuid;

#[path = "../common.rs"]
mod common;

/// Upper bound for one USDA call (an external government API can be slow);
/// exceeding it fails the test.
const USDA_API_TIMEOUT: Duration = Duration::from_secs(30);

/// Executor whose config carries the real `USDA_API_KEY`, injected
/// explicitly rather than inherited from whatever the default test config
/// read.
async fn create_usda_live_executor() -> Result<UniversalToolExecutor> {
    common::init_server_config();
    common::init_test_http_clients();

    let api_key = env::var("USDA_API_KEY")
        .expect("USDA_API_KEY must be set to run usda_live_test (USDA FoodData Central key)");
    assert!(!api_key.is_empty(), "USDA_API_KEY is set but empty");

    let config = ServerConfig {
        usda_api_key: Some(api_key),
        ..common::test_server_config()
    };
    let resources = common::create_test_server_resources_with_config(config).await?;
    Ok(UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()))
}

/// Request without a tenant (the nutrition tools are tenant-agnostic).
fn create_nutrition_request(tool_name: &str, parameters: serde_json::Value) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool_name.to_owned(),
        parameters,
        user_id: Uuid::new_v4().to_string(),
        protocol: "test".to_owned(),
        tenant_id: None,
    }
}

/// Create a user with a provisioned tenant; `validate_recipe` requires an
/// explicit `tenant_id` on every request.
async fn create_recipe_user(executor: &UniversalToolExecutor) -> Result<(Uuid, Uuid)> {
    let user = User::new(
        format!("usda_live_{}@example.com", Uuid::new_v4()),
        "password_hash".to_owned(),
        Some("USDA Live Test User".to_owned()),
    );
    let user_id = user.id;
    executor.resources.repos().users.create(&user).await?;

    let tenant = Tenant::new(
        "USDA Live Test Tenant".to_owned(),
        format!("usda-live-tenant-{}", Uuid::new_v4()),
        Some(format!("{}.example.com", Uuid::new_v4())),
        "starter".to_owned(),
        user_id,
    );
    let tenant_id: Uuid = tenant.id.into();
    executor.resources.repos().tenants.create(&tenant).await?;

    Ok((user_id, tenant_id))
}

/// Execute a USDA-backed tool call and require a successful answer.
///
/// A timeout, a transport error or an unsuccessful response — rate limit and
/// 5xx included — fails the test with the error USDA returned.
async fn execute_usda_call(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Result<UniversalResponse> {
    let tool = request.tool_name.clone();
    let response = timeout(USDA_API_TIMEOUT, executor.execute_tool(request))
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{tool}: USDA did not answer within {} seconds",
                USDA_API_TIMEOUT.as_secs()
            )
        })?;
    assert!(
        response.success,
        "{tool} failed against live USDA: {}",
        response.error.as_deref().unwrap_or("no error message")
    );
    Ok(response)
}

// ============================================================================
// search_food
// ============================================================================

#[tokio::test]
async fn test_search_food_with_api_key() -> Result<()> {
    let executor = create_usda_live_executor().await?;

    let request = create_nutrition_request(
        "search_food",
        json!({
            "query": "chicken breast raw",
            "page_size": 5
        }),
    );

    let response = execute_usda_call(&executor, request).await?;
    let result = response.result.unwrap();

    assert!(result["foods"].is_array(), "Should return foods array");
    // Response uses total_hits for total result count, returned_count for items in response
    assert!(
        result["total_hits"].as_u64().unwrap_or(0) > 0
            || result["returned_count"].as_u64().unwrap_or(0) > 0,
        "Should find results"
    );

    Ok(())
}

#[tokio::test]
async fn test_search_food_pagination_metadata_fields() -> Result<()> {
    let executor = create_usda_live_executor().await?;

    let request = create_nutrition_request(
        "search_food",
        json!({
            "query": "chicken",
            "page_size": 5,
            "page_number": 1
        }),
    );

    let response = execute_usda_call(&executor, request).await?;
    let result = response.result.unwrap();

    // Verify all pagination metadata fields are present
    assert!(
        result.get("returned_count").is_some(),
        "Response should include returned_count"
    );
    assert!(
        result.get("total_hits").is_some(),
        "Response should include total_hits"
    );
    assert!(
        result.get("page_number").is_some(),
        "Response should include page_number"
    );
    assert!(
        result.get("page_size").is_some(),
        "Response should include page_size"
    );
    assert!(
        result.get("total_pages").is_some(),
        "Response should include total_pages"
    );
    assert!(
        result.get("has_more").is_some(),
        "Response should include has_more"
    );

    // Verify page_number matches request
    assert_eq!(result["page_number"].as_u64().unwrap(), 1);
    assert_eq!(result["page_size"].as_u64().unwrap(), 5);

    Ok(())
}

#[tokio::test]
async fn test_search_food_pagination_has_more_calculation() -> Result<()> {
    let executor = create_usda_live_executor().await?;

    // A page size of 2 for a common food guarantees several pages exist
    let request = create_nutrition_request(
        "search_food",
        json!({
            "query": "apple",
            "page_size": 2,
            "page_number": 1
        }),
    );

    let response = execute_usda_call(&executor, request).await?;
    let result = response.result.unwrap();

    let current_page = result["page_number"].as_u64().unwrap();
    let total_pages = result["total_pages"].as_u64().unwrap();
    let has_more = result["has_more"].as_bool().unwrap();

    assert!(
        total_pages > 1,
        "USDA should report more than one page of 'apple' at page_size 2, got {total_pages}"
    );
    // Verify has_more is calculated correctly: current_page < total_pages
    assert!(
        has_more,
        "has_more should be true when on page {current_page} of {total_pages}"
    );

    Ok(())
}

#[tokio::test]
async fn test_search_food_pagination_page_navigation() -> Result<()> {
    let executor = create_usda_live_executor().await?;

    let request1 = create_nutrition_request(
        "search_food",
        json!({
            "query": "beef",
            "page_size": 3,
            "page_number": 1
        }),
    );

    let response1 = execute_usda_call(&executor, request1).await?;
    let result1 = response1.result.unwrap();
    assert_eq!(result1["page_number"].as_u64().unwrap(), 1);
    assert!(
        result1["has_more"].as_bool().unwrap_or(false),
        "USDA should report a second page of 'beef' at page_size 3"
    );

    let request2 = create_nutrition_request(
        "search_food",
        json!({
            "query": "beef",
            "page_size": 3,
            "page_number": 2
        }),
    );

    let response2 = execute_usda_call(&executor, request2).await?;
    let result2 = response2.result.unwrap();
    assert_eq!(
        result2["page_number"].as_u64().unwrap(),
        2,
        "page_number should reflect requested page"
    );

    Ok(())
}

// ============================================================================
// get_food_details
// ============================================================================

#[tokio::test]
async fn test_get_food_details_with_api_key() -> Result<()> {
    let executor = create_usda_live_executor().await?;

    // Use a known FDC ID (chicken breast)
    let request = create_nutrition_request(
        "get_food_details",
        json!({
            "fdc_id": 171_477
        }),
    );

    let response = execute_usda_call(&executor, request).await?;
    let result = response.result.unwrap();

    assert!(result["fdc_id"].as_u64().is_some(), "Should have fdc_id");
    assert!(result["description"].is_string(), "Should have description");
    assert!(
        result["nutrients"].is_array(),
        "Should have nutrients array"
    );

    Ok(())
}

// ============================================================================
// analyze_meal_nutrition
// ============================================================================

#[tokio::test]
async fn test_analyze_meal_nutrition_with_api_key() -> Result<()> {
    let executor = create_usda_live_executor().await?;

    let request = create_nutrition_request(
        "analyze_meal_nutrition",
        json!({
            "ingredients": [
                {"fdc_id": 171_477, "amount_g": 150.0}
            ]
        }),
    );

    let response = execute_usda_call(&executor, request).await?;
    let result = response.result.unwrap();

    assert!(
        result["total_calories"].as_f64().unwrap() > 0.0,
        "Should have calories"
    );
    assert!(
        result["total_protein_g"].as_f64().is_some(),
        "Should have protein"
    );
    assert!(result["foods"].is_array(), "Should have foods array");

    Ok(())
}

// ============================================================================
// validate_recipe
// ============================================================================

#[tokio::test]
async fn test_validate_recipe_with_api_key() -> Result<()> {
    let executor = create_usda_live_executor().await?;
    let (user_id, tenant_id) = create_recipe_user(&executor).await?;

    let request = UniversalRequest {
        tool_name: "validate_recipe".to_owned(),
        parameters: json!({
            "name": "Chicken and Rice",
            "servings": 4,
            "ingredients": [
                {"name": "chicken breast", "amount": 500.0, "unit": "grams"},
                {"name": "white rice", "amount": 300.0, "unit": "grams"}
            ]
        }),
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
    };

    let response = execute_usda_call(&executor, request).await?;
    let result = response.result.unwrap();

    assert!(result["validated"].as_bool().unwrap());
    assert!(result["nutrition_per_serving"].is_object());

    // Zero matches for chicken breast + white rice is a degraded USDA answer,
    // and a degraded answer fails the test.
    let matched = result["usda_matched_count"]
        .as_u64()
        .expect("usda_matched_count must be reported");
    assert!(
        matched > 0,
        "USDA returned 0 ingredient matches for chicken breast + white rice"
    );

    // A matched ingredient with zero calories is a parse failure, not a
    // degraded API: the client asks the detail endpoint for its nested shape
    // and fails closed on a nutrient list that names no nutrient, so a detail
    // response it cannot read surfaces as an unmatched ingredient above, never
    // as a silent zero here (carnet#423).
    let calories = result["nutrition_per_serving"]["calories"]
        .as_f64()
        .expect("calories must be reported as a number");
    assert!(
        calories > 0.0,
        "USDA matched {matched} ingredients but reported zero calories"
    );
    assert!(result["validation_completeness"].as_f64().is_some());

    Ok(())
}
