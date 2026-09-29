// ABOUTME: Tests typed-parameter deserialization for the goal tool params in json_schemas
// ABOUTME: Asserts serde::from_value reads each field and applies the declared defaults
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used, clippy::float_cmp)]

use pierre_mcp_schema::json_schemas::{AnalyzeGoalFeasibilityParams, SetGoalParams};
use serde_json::json;

#[test]
fn test_goals_params_deserialization() {
    // Test AnalyzeGoalFeasibilityParams
    let params_json = json!({"goal_type": "distance", "target_value": 42.195});
    let params: AnalyzeGoalFeasibilityParams = serde_json::from_value(params_json).unwrap();
    assert_eq!(params.goal_type, "distance");
    assert_eq!(params.target_value, 42.195);

    // Test SetGoalParams
    let params_json = json!({
        "goal_type": "duration",
        "target_value": 60.0,
        "timeframe": "week"
    });
    let params: SetGoalParams = serde_json::from_value(params_json).unwrap();
    assert_eq!(params.goal_type, "duration");
    assert_eq!(params.target_value, 60.0);
    assert_eq!(params.timeframe, "week");
    assert_eq!(params.title, "Fitness Goal"); // default
}

/// `set_goal` declares only `goal_type` and `target_value` required, so a call
/// naming no timeframe has to deserialize: it sets a monthly goal. It used to
/// fail with "missing field `timeframe`" while the schema promised a default.
#[test]
fn set_goal_params_default_to_a_monthly_goal_counting_every_sport() {
    let params: SetGoalParams =
        serde_json::from_value(json!({"goal_type": "frequency", "target_value": 12.0})).unwrap();
    assert_eq!(params.timeframe, "month");
    assert_eq!(params.sport, None);

    let params: SetGoalParams = serde_json::from_value(json!({
        "goal_type": "distance",
        "target_value": 200.0,
        "sport": "ride"
    }))
    .unwrap();
    assert_eq!(params.sport.as_deref(), Some("ride"));
}
