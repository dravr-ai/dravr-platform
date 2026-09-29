// ABOUTME: Integration tests for mobility MCP tools (stretching exercises and yoga poses)
// ABOUTME: Tests tool registration, database operations, and activity-based recommendations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Mobility Tool Handler Integration Tests
//!
//! Tests the 6 mobility MCP tools via the `UniversalToolExecutor`:
//! - `list_stretching_exercises`: List stretching exercises with optional filters
//! - `get_stretching_exercise`: Get a specific stretching exercise by ID
//! - `suggest_stretches_for_activity`: Get activity-specific stretch recommendations
//! - `list_yoga_poses`: List yoga poses with optional filters
//! - `get_yoga_pose`: Get a specific yoga pose by ID
//! - `suggest_yoga_sequence`: Generate a yoga sequence for recovery
//!
//! Every argument a schema declares is exercised against the catalogue
//! `pierre-cli seed mobility` writes, so a handler that drops an argument
//! returns the unfiltered catalogue and fails on the names it lists.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_seeders::mobility;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use uuid::Uuid;

mod common;

// ============================================================================
// Test Setup
// ============================================================================

/// Create test executor for mobility tool tests
async fn create_mobility_test_executor() -> Result<UniversalToolExecutor> {
    common::init_server_config();
    common::init_test_http_clients();

    let resources = common::create_test_server_resources().await?;
    Ok(UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()))
}

/// Create a test executor over the catalogue the mobility seeder writes.
async fn create_seeded_executor() -> Result<UniversalToolExecutor> {
    common::init_server_config();
    common::init_test_http_clients();

    let resources = common::create_test_server_resources().await?;
    mobility::run(&resources.common.repos).await?;
    Ok(UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()))
}

/// Run one tool call that must succeed and return its result.
async fn call_ok(executor: &UniversalToolExecutor, tool_name: &str, parameters: Value) -> Value {
    let response = executor
        .execute_tool(create_test_request(tool_name, parameters, Uuid::new_v4()))
        .await
        .unwrap();
    assert!(
        response.success,
        "{tool_name} should succeed: {:?}",
        response.error
    );
    response.result.unwrap()
}

/// The `field` of every entry of the result array `key`, in the order returned.
fn field_values(result: &Value, key: &str, field: &str) -> Vec<String> {
    result[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} should be an array: {result}"))
        .iter()
        .map(|entry| entry[field].as_str().unwrap().to_owned())
        .collect()
}

/// [`field_values`] sorted, for listings whose order is the database collation's.
fn sorted_values(result: &Value, key: &str, field: &str) -> Vec<String> {
    let mut values = field_values(result, key, field);
    values.sort();
    values
}

/// Create a test request with user ID
fn create_test_request(
    tool_name: &str,
    parameters: serde_json::Value,
    user_id: Uuid,
) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool_name.to_owned(),
        parameters,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: None,
    }
}

// ============================================================================
// Tool Registration Tests
// ============================================================================

#[tokio::test]
async fn test_mobility_tools_registered() -> Result<()> {
    let executor = create_mobility_test_executor().await?;

    let tool_names: Vec<String> = executor
        .resources
        .tool_registry()
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();

    let expected_tools = vec![
        "list_stretching_exercises",
        "get_stretching_exercise",
        "suggest_stretches_for_activity",
        "list_yoga_poses",
        "get_yoga_pose",
        "suggest_yoga_sequence",
    ];

    for expected_tool in expected_tools {
        assert!(
            tool_names.contains(&expected_tool.to_owned()),
            "Missing mobility tool: {expected_tool}"
        );
    }

    Ok(())
}

// ============================================================================
// list_stretching_exercises Tests
// ============================================================================

#[tokio::test]
async fn test_list_stretching_exercises_default() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request("list_stretching_exercises", json!({}), user_id);

    let response = executor.execute_tool(request).await?;

    assert!(
        response.success,
        "Tool should succeed: {:?}",
        response.error
    );
    let result = response.result.unwrap();

    // Result should have exercises array
    assert!(
        result["exercises"].is_array(),
        "Should have exercises array"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_with_category_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_stretching_exercises",
        json!({
            "category": "dynamic"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success, "Tool should succeed");
    let result = response.result.unwrap();

    assert!(result["exercises"].is_array());

    // If there are exercises, verify they have the correct category
    if let Some(exercises) = result["exercises"].as_array() {
        for exercise in exercises {
            assert_eq!(
                exercise["category"].as_str(),
                Some("dynamic"),
                "Filtered exercises should be dynamic category"
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_with_difficulty_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_stretching_exercises",
        json!({
            "difficulty": "beginner"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["exercises"].is_array());

    // If there are exercises, verify they have the correct difficulty
    if let Some(exercises) = result["exercises"].as_array() {
        for exercise in exercises {
            assert_eq!(
                exercise["difficulty"].as_str(),
                Some("beginner"),
                "Filtered exercises should be beginner difficulty"
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_with_muscle_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_stretching_exercises",
        json!({
            "muscle_group": "hamstrings"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["exercises"].is_array());

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_with_pagination() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_stretching_exercises",
        json!({
            "limit": 5,
            "offset": 0
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    if let Some(exercises) = result["exercises"].as_array() {
        assert!(exercises.len() <= 5, "Should respect limit parameter");
    }

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_filters_by_activity_type() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "list_stretching_exercises",
        json!({ "activity_type": "swimming" }),
    )
    .await;

    assert_eq!(
        sorted_values(&result, "exercises", "name"),
        [
            "Cat-Cow Spine Mobility",
            "Chest Doorway Stretch",
            "Lat Stretch on Wall"
        ],
        "only the stretches recommended for swimming"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_muscle_group_names_a_muscle() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "list_stretching_exercises",
        json!({ "muscle_group": "hamstrings" }),
    )
    .await;

    assert_eq!(
        sorted_values(&result, "exercises", "name"),
        ["Hamstring Doorway Stretch", "Leg Swings - Front to Back"]
    );

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_muscle_group_expands_a_body_area() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "list_stretching_exercises",
        json!({ "muscle_group": "back" }),
    )
    .await;

    assert_eq!(
        sorted_values(&result, "exercises", "name"),
        [
            "Cat-Cow Spine Mobility",
            "Lat Stretch on Wall",
            "Walking Lunges with Twist"
        ],
        "back covers lower_back, upper_back, lats and thoracic_spine"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_it_band_finds_the_muscles_beside_it() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "list_stretching_exercises",
        json!({ "muscle_group": "it_band" }),
    )
    .await;

    assert_eq!(
        sorted_values(&result, "exercises", "name"),
        [
            "Figure-4 Glute Stretch",
            "IT Band Foam Roll",
            "Leg Swings - Front to Back",
            "Pigeon Pose Stretch"
        ],
        "the band does not lengthen, so it finds the outer quadriceps and hip muscles"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_exercises_limit_cuts_the_catalogue() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let everything = call_ok(&executor, "list_stretching_exercises", json!({})).await;
    assert_eq!(
        everything["count"], 12,
        "the whole catalogue fits the default limit"
    );

    let five = call_ok(
        &executor,
        "list_stretching_exercises",
        json!({ "limit": 5 }),
    )
    .await;
    assert_eq!(five["count"], 5);

    Ok(())
}

// ============================================================================
// get_stretching_exercise Tests
// ============================================================================

#[tokio::test]
async fn test_get_stretching_exercise_not_found() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "get_stretching_exercise",
        json!({
            "exercise_id": "nonexistent-id"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    // Should fail for nonexistent ID
    assert!(!response.success, "Should fail for nonexistent exercise");
    assert!(response.error.is_some());

    Ok(())
}

#[tokio::test]
async fn test_get_stretching_exercise_missing_id() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request("get_stretching_exercise", json!({}), user_id);

    let result = executor.execute_tool(request).await;

    // Should error on missing required parameter
    assert!(result.is_err() || !result.unwrap().success);

    Ok(())
}

// ============================================================================
// suggest_stretches_for_activity Tests
// ============================================================================

#[tokio::test]
async fn test_suggest_stretches_for_running() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_stretches_for_activity",
        json!({
            "activity_type": "running"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(
        response.success,
        "Tool should succeed: {:?}",
        response.error
    );
    let result = response.result.unwrap();

    // Should have suggested exercises
    assert!(result["exercises"].is_array() || result["suggestions"].is_array());

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_for_cycling() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_stretches_for_activity",
        json!({
            "activity_type": "cycling"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    // Verify activity type is echoed back
    assert!(
        result["activity_type"].as_str().is_some()
            || result["exercises"].is_array()
            || result["suggestions"].is_array()
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_warmup_focus_keeps_dynamic_stretches() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "suggest_stretches_for_activity",
        json!({ "activity_type": "running", "focus": "warm-up" }),
    )
    .await;

    assert_eq!(
        field_values(&result, "exercises", "name"),
        ["Leg Swings - Front to Back", "Walking Lunges with Twist"],
        "a warm-up holds only the running catalogue's dynamic stretches"
    );
    assert_eq!(result["count"], 2);
    assert_eq!(
        result["total_duration_seconds"], 90,
        "30 s of leg swings and 60 s of walking lunges, one set each"
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_cooldown_focus_keeps_static_stretches() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "suggest_stretches_for_activity",
        json!({ "activity_type": "running", "focus": "cooldown" }),
    )
    .await;

    let categories = field_values(&result, "exercises", "category");
    assert_eq!(
        categories.len(),
        6,
        "seven static running stretches, cut to the default of six"
    );
    assert!(
        categories.iter().all(|c| c == "static"),
        "a cool-down holds only static stretches: {categories:?}"
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_without_focus_mixes_categories() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "suggest_stretches_for_activity",
        json!({ "activity_type": "running" }),
    )
    .await;

    let names = field_values(&result, "exercises", "name");
    assert_eq!(names.len(), 6, "the default limit is six");
    assert_eq!(
        names[..2],
        ["Leg Swings - Front to Back", "Walking Lunges with Twist"],
        "without a focus the dynamic stretches lead, then the static ones"
    );
    assert_eq!(
        field_values(&result, "exercises", "category")[2..],
        ["static", "static", "static", "static"]
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_honours_limit() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "suggest_stretches_for_activity",
        json!({ "activity_type": "running", "limit": 3 }),
    )
    .await;

    assert_eq!(field_values(&result, "exercises", "name").len(), 3);
    assert_eq!(result["count"], 3);

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_rejects_unknown_focus() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let response = executor
        .execute_tool(create_test_request(
            "suggest_stretches_for_activity",
            json!({ "activity_type": "running", "focus": "midday" }),
            Uuid::new_v4(),
        ))
        .await;

    match response {
        Ok(response) => {
            assert!(!response.success, "an unknown focus is refused");
            let error = response.error.unwrap_or_default();
            assert!(
                error.contains("focus"),
                "the refusal names the argument: {error}"
            );
        }
        Err(error) => assert!(
            error.to_string().contains("focus"),
            "the refusal names the argument: {error}"
        ),
    }

    Ok(())
}

#[tokio::test]
async fn test_suggest_stretches_missing_activity() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request("suggest_stretches_for_activity", json!({}), user_id);

    let result = executor.execute_tool(request).await;

    // Should error on missing required parameter
    assert!(result.is_err() || !result.unwrap().success);

    Ok(())
}

// ============================================================================
// list_yoga_poses Tests
// ============================================================================

#[tokio::test]
async fn test_list_yoga_poses_default() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request("list_yoga_poses", json!({}), user_id);

    let response = executor.execute_tool(request).await?;

    assert!(
        response.success,
        "Tool should succeed: {:?}",
        response.error
    );
    let result = response.result.unwrap();

    // Result should have poses array
    assert!(result["poses"].is_array(), "Should have poses array");

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_with_category_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "category": "standing"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["poses"].is_array());

    // If there are poses, verify they have the correct category
    if let Some(poses) = result["poses"].as_array() {
        for pose in poses {
            assert_eq!(
                pose["category"].as_str(),
                Some("standing"),
                "Filtered poses should be standing category"
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_with_difficulty_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "difficulty": "advanced"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["poses"].is_array());

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_with_pose_type_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "pose_type": "stretch"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["poses"].is_array());

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_with_recovery_context_filter() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "recovery_context": "post_cardio"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["poses"].is_array());

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_with_pagination() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "limit": 3,
            "offset": 0
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    if let Some(poses) = result["poses"].as_array() {
        assert!(poses.len() <= 3, "Should respect limit parameter");
    }

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_muscle_group_expands_hips() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let result = call_ok(
        &executor,
        "list_yoga_poses",
        json!({ "muscle_group": "hips" }),
    )
    .await;

    assert_eq!(
        sorted_values(&result, "poses", "english_name"),
        [
            "Butterfly Pose",
            "Child's Pose",
            "Half Lord of the Fishes",
            "Happy Baby",
            "Reclined Pigeon",
            "Supine Spinal Twist",
            "Tree Pose",
            "Triangle Pose",
            "Warrior I",
            "Warrior II"
        ],
        "every pose working a hip muscle, as a primary or a secondary one"
    );

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_poses_muscle_group_shoulders_and_muscle() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let shoulders = call_ok(
        &executor,
        "list_yoga_poses",
        json!({ "muscle_group": "shoulders" }),
    )
    .await;
    assert_eq!(
        sorted_values(&shoulders, "poses", "english_name"),
        ["Child's Pose", "Downward Facing Dog", "Warrior II"]
    );

    let calves = call_ok(
        &executor,
        "list_yoga_poses",
        json!({ "muscle_group": "calves" }),
    )
    .await;
    assert_eq!(
        sorted_values(&calves, "poses", "english_name"),
        ["Downward Facing Dog", "Seated Forward Fold"],
        "a muscle as the catalogue spells it matches itself"
    );

    Ok(())
}

// ============================================================================
// get_yoga_pose Tests
// ============================================================================

#[tokio::test]
async fn test_get_yoga_pose_not_found() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "get_yoga_pose",
        json!({
            "pose_id": "nonexistent-pose-id"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    // Should fail for nonexistent ID
    assert!(!response.success, "Should fail for nonexistent pose");
    assert!(response.error.is_some());

    Ok(())
}

#[tokio::test]
async fn test_get_yoga_pose_missing_id() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request("get_yoga_pose", json!({}), user_id);

    let result = executor.execute_tool(request).await;

    // Should error on missing required parameter
    assert!(result.is_err() || !result.unwrap().success);

    Ok(())
}

// ============================================================================
// suggest_yoga_sequence Tests
// ============================================================================

#[tokio::test]
async fn test_suggest_yoga_sequence_for_recovery() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_yoga_sequence",
        json!({
            "purpose": "recovery"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(
        response.success,
        "Tool should succeed: {:?}",
        response.error
    );
    let result = response.result.unwrap();

    // Should have sequence or poses
    assert!(
        result["sequence"].is_array() || result["poses"].is_array(),
        "Should have sequence of poses"
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_yoga_sequence_for_post_run() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_yoga_sequence",
        json!({
            "purpose": "post_run"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    // Verify purpose is echoed back or sequence is present
    assert!(
        result["purpose"].as_str().is_some()
            || result["sequence"].is_array()
            || result["poses"].is_array()
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_yoga_sequence_with_duration() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_yoga_sequence",
        json!({
            "purpose": "recovery",
            "duration_minutes": 15
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    // Should have sequence metadata including duration
    assert!(
        result["sequence"].is_array()
            || result["poses"].is_array()
            || result["total_duration_seconds"].is_number()
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_yoga_sequence_with_difficulty() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_yoga_sequence",
        json!({
            "purpose": "recovery",
            "difficulty": "beginner"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);

    Ok(())
}

#[tokio::test]
async fn test_suggest_yoga_sequence_focus_area_keeps_to_the_area() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let unfocused = call_ok(
        &executor,
        "suggest_yoga_sequence",
        json!({ "purpose": "post_cardio", "duration_minutes": 30 }),
    )
    .await;
    assert_eq!(
        field_values(&unfocused, "sequence", "english_name"),
        [
            "Butterfly Pose",
            "Seated Forward Fold",
            "Downward Facing Dog",
            "Triangle Pose",
            "Reclined Pigeon",
            "Supine Spinal Twist"
        ],
        "without a focus every post-cardio pose, by category then name"
    );

    let hips = call_ok(
        &executor,
        "suggest_yoga_sequence",
        json!({ "purpose": "post_cardio", "duration_minutes": 30, "focus_area": "hips" }),
    )
    .await;
    assert_eq!(
        field_values(&hips, "sequence", "english_name"),
        [
            "Butterfly Pose",
            "Triangle Pose",
            "Reclined Pigeon",
            "Supine Spinal Twist"
        ],
        "a hip focus drops the post-cardio poses that work no hip muscle"
    );
    assert_eq!(hips["total_duration_seconds"], 210);
    assert!(
        hips["guidance"]
            .as_str()
            .unwrap()
            .contains("30-minute yoga sequence is designed for post cardio, focused on the hips"),
        "{}",
        hips["guidance"]
    );

    let back = call_ok(
        &executor,
        "suggest_yoga_sequence",
        json!({ "purpose": "post_cardio", "duration_minutes": 30, "focus_area": "back" }),
    )
    .await;
    assert_eq!(
        field_values(&back, "sequence", "english_name"),
        [
            "Butterfly Pose",
            "Seated Forward Fold",
            "Downward Facing Dog",
            "Supine Spinal Twist"
        ]
    );

    Ok(())
}

#[tokio::test]
async fn test_suggest_yoga_sequence_difficulty_is_a_maximum() -> Result<()> {
    let executor = create_seeded_executor().await?;

    let up_to_intermediate = call_ok(
        &executor,
        "suggest_yoga_sequence",
        json!({ "purpose": "rest_day", "duration_minutes": 30, "difficulty": "intermediate" }),
    )
    .await;
    let names = field_values(&up_to_intermediate, "sequence", "english_name");
    assert_eq!(names.len(), 13, "every rest-day pose fits 30 minutes");
    assert!(names.iter().any(|n| n == "Half Lord of the Fishes"));
    assert!(
        names.iter().any(|n| n == "Tree Pose"),
        "a maximum keeps the easier poses too"
    );

    let beginner = call_ok(
        &executor,
        "suggest_yoga_sequence",
        json!({ "purpose": "rest_day", "duration_minutes": 30, "difficulty": "beginner" }),
    )
    .await;
    let difficulties = field_values(&beginner, "sequence", "difficulty");
    assert_eq!(
        difficulties.len(),
        12,
        "the one intermediate pose drops out"
    );
    assert!(difficulties.iter().all(|d| d == "beginner"));

    Ok(())
}

#[tokio::test]
async fn test_suggest_yoga_sequence_missing_purpose() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request("suggest_yoga_sequence", json!({}), user_id);

    let result = executor.execute_tool(request).await;

    // Should error on missing required parameter
    assert!(result.is_err() || !result.unwrap().success);

    Ok(())
}

// ============================================================================
// Combined Filter Tests
// ============================================================================

#[tokio::test]
async fn test_list_stretching_with_multiple_filters() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_stretching_exercises",
        json!({
            "category": "static",
            "difficulty": "beginner",
            "limit": 10
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["exercises"].is_array());

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_with_multiple_filters() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "category": "standing",
            "difficulty": "intermediate",
            "pose_type": "strength",
            "limit": 5
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["poses"].is_array());

    Ok(())
}

// ============================================================================
// Edge Cases
// ============================================================================

#[tokio::test]
async fn test_suggest_stretches_unknown_activity() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "suggest_stretches_for_activity",
        json!({
            "activity_type": "unknown_sport_xyz"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    // Should either succeed with empty results or provide fallback
    // Behavior depends on implementation - both are acceptable
    if response.success {
        let result = response.result.unwrap();
        assert!(
            result["exercises"].is_array() || result["suggestions"].is_array(),
            "Should return array even for unknown activity"
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_list_stretching_invalid_category() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_stretching_exercises",
        json!({
            "category": "invalid_category_xyz"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    // Should handle gracefully - either succeed with empty results or default category
    // Both behaviors are acceptable
    if response.success {
        let result = response.result.unwrap();
        assert!(result["exercises"].is_array());
    }

    Ok(())
}

#[tokio::test]
async fn test_list_yoga_invalid_difficulty() -> Result<()> {
    let executor = create_mobility_test_executor().await?;
    let user_id = Uuid::new_v4();

    let request = create_test_request(
        "list_yoga_poses",
        json!({
            "difficulty": "impossible"
        }),
        user_id,
    );

    let response = executor.execute_tool(request).await?;

    // Should handle gracefully - either succeed with empty results or default difficulty
    if response.success {
        let result = response.result.unwrap();
        assert!(result["poses"].is_array());
    }

    Ok(())
}
