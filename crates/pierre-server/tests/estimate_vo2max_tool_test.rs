// ABOUTME: Exercises estimate_vo2max end to end through the tool executor — each field test against its published equation
// ABOUTME: Pins the profile-default path, the rejection messages, and that the tool is reachable from a chat turn

// SPDX-License-Identifier: MIT OR Apache-2.0

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::scopes::{missing_scope, required_scopes};
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    // The dispatch chokepoint enforces OAuth scopes; the seeding write
    // (set_physiology) needs the athlete's own grant, exactly as its tests do.
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("vo2max_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn make_request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

async fn estimate(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<Value> {
    let response = executor
        .execute_tool(make_request("estimate_vo2max", params, user_id, tenant_id))
        .await?;
    assert!(
        response.success,
        "estimate_vo2max should have succeeded: {:?}",
        response.error
    );
    Ok(response.result.expect("estimate_vo2max returns a payload"))
}

/// The text of the rejection, from whichever channel it arrives on.
async fn estimate_expecting_rejection(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    params: Value,
) -> Result<String> {
    match executor
        .execute_tool(make_request("estimate_vo2max", params, user_id, tenant_id))
        .await
    {
        Err(e) => Ok(e.to_string()),
        Ok(response) => {
            assert!(
                !response.success,
                "estimate_vo2max should have rejected these values, got: {:?}",
                response.result
            );
            Ok(format!("{:?}", response.error))
        }
    }
}

fn vo2(result: &Value) -> f64 {
    result["vo2max_ml_kg_min"]
        .as_f64()
        .expect("vo2max_ml_kg_min is a number")
}

// ============================================================================
// Each method reproduces its published equation
// ============================================================================

#[tokio::test]
async fn cooper_reports_the_published_equation() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // Cooper (1968): VO2max = (distance_m − 504.9) / 44.73. 2,800 m → 51.31.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "cooper_test", "distance_meters": 2800 }),
    )
    .await?;

    let expected = (2800.0_f64 - 504.9) / 44.73;
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "Cooper for 2,800 m should be {expected:.2}, got {}",
        vo2(&result)
    );
    assert_eq!(result["method"], json!("cooper_test"));
    assert_eq!(
        result["saved"],
        json!(false),
        "the tool estimates, it does not write"
    );
    assert_eq!(result["defaults_from_profile"], json!([]));
    assert!(
        result["formula"].as_str().unwrap_or("").contains("Cooper"),
        "the formula field names the method's source"
    );
    Ok(())
}

#[tokio::test]
async fn from_vdot_reports_the_vdot_unchanged() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // VDOT is already ml/kg/min. This pins the retired ×3.5 MET conversion
    // dead: a 50 that came back as 175 would fail here.
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "from_vdot", "vdot": 50 }),
    )
    .await?;
    assert!(
        (vo2(&result) - 50.0).abs() < f64::EPSILON,
        "got {}",
        vo2(&result)
    );
    Ok(())
}

// ============================================================================
// Profile defaults
// ============================================================================

#[tokio::test]
async fn rockport_takes_weight_and_age_from_the_profile_and_says_so() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "weight": 70.0, "age": 30 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(
        saved.success,
        "seeding the profile failed: {:?}",
        saved.error
    );

    // Kline et al. (1987), as cageux implements it with time in minutes:
    // 132.853 − 0.0769·kg − 0.3877·age + 6.315·gender − 3.2649·min − 0.1565·HR
    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "male",
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;

    // Summed term by term so the test reads like the equation; clippy's
    // mul_add preference does not apply to a plain sum.
    let expected: f64 = [
        132.853,
        -0.0769 * 70.0,
        -0.3877 * 30.0,
        6.315,
        -3.2649 * 13.0,
        -0.1565 * 140.0,
    ]
    .iter()
    .sum();
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "Rockport should be {expected:.2}, got {}",
        vo2(&result)
    );
    assert_eq!(
        result["defaults_from_profile"],
        json!(["weight_kg", "age"]),
        "the response must name every input that came from the profile"
    );
    Ok(())
}

#[tokio::test]
async fn a_restated_weight_overrides_the_profile() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let saved = executor
        .execute_tool(make_request(
            "set_physiology",
            json!({ "weight": 90.0, "age": 30 }),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(saved.success);

    let result = estimate(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "female",
            "weight_kg": 60.0,
            "time_seconds": 900,
            "heart_rate": 150,
        }),
    )
    .await?;

    // Weight from the call (60), age from the profile (30), gender 0.
    let expected: f64 = [
        132.853,
        -0.0769 * 60.0,
        -0.3877 * 30.0,
        -3.2649 * 15.0,
        -0.1565 * 150.0,
    ]
    .iter()
    .sum();
    assert!(
        (vo2(&result) - expected).abs() < 0.06,
        "got {}",
        vo2(&result)
    );
    assert_eq!(result["defaults_from_profile"], json!(["age"]));
    Ok(())
}

// ============================================================================
// Rejections say what to do
// ============================================================================

#[tokio::test]
async fn rockport_with_no_weight_anywhere_points_at_set_physiology() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "male",
            "age": 30,
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;
    assert!(
        message.contains("weight_kg"),
        "names the missing field: {message}"
    );
    assert!(
        message.contains("set_physiology"),
        "tells the coach how to fix it: {message}"
    );
    Ok(())
}

#[tokio::test]
async fn an_unknown_method_lists_the_real_ones() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "treadmill", "distance_meters": 2800 }),
    )
    .await?;
    assert!(message.contains("cooper_test"), "{message}");
    assert!(message.contains("rockport_walk"), "{message}");
    Ok(())
}

#[tokio::test]
async fn gender_outside_the_fitted_equation_is_rejected() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({
            "method": "rockport_walk",
            "gender": "other",
            "weight_kg": 70,
            "age": 30,
            "time_seconds": 780,
            "heart_rate": 140,
        }),
    )
    .await?;
    assert!(message.contains("female or male"), "{message}");
    Ok(())
}

#[tokio::test]
async fn an_out_of_range_input_is_the_athletes_to_correct() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;

    // cageux bounds VDOT to the 30–85 range the tables cover.
    let message = estimate_expecting_rejection(
        &executor,
        user_id,
        &tenant_id,
        json!({ "method": "from_vdot", "vdot": 200 }),
    )
    .await?;
    assert!(
        message.contains("cannot estimate VO2max"),
        "an estimator error surfaces as invalid input, not a server fault: {message}"
    );
    Ok(())
}

// ============================================================================
// Reachability
// ============================================================================

#[test]
fn estimate_vo2max_is_reachable_from_a_chat_turn() {
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let names: Vec<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(
        names.iter().any(|n| n == "estimate_vo2max"),
        "estimate_vo2max must sit in a chat-callable category or the coach can never see it; got {} tools",
        names.len()
    );
}

// ============================================================================
// The profile access is scope-gated (carnet#363)
// ============================================================================

#[test]
fn the_tool_declares_the_profile_access_it_performs() {
    // reads the stored profile for weight and age defaults and echoes `stored_vo2_max`,
    // so the call touches identity data. Declaring only the runtime
    // requirements yields an empty scope list, which the
    // read-only default grant an RFC 7591 registration receives satisfies —
    // and that grant is exactly what the profile split exists to hold back.
    common::init_server_config();
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let (_, _, caps, _) = registry
        .all_tool_metadata()
        .into_iter()
        .find(|(name, _, _, _)| name == "estimate_vo2max")
        .expect("the tool is registered");

    assert!(
        required_scopes(caps).contains(&OAuthScope::ProfileRead),
        "touching the stored profile requires profile:read, not fitness:read"
    );
    assert_eq!(
        missing_scope(&[OAuthScope::FitnessRead], caps),
        Some(OAuthScope::ProfileRead),
        "a fitness-read grant must be refused and told what it needed"
    );
    assert_eq!(
        missing_scope(&OAuthScope::self_grant(), caps),
        None,
        "the athlete's own grant covers it"
    );
}
