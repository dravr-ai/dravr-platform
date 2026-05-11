// ABOUTME: Integration tests for endurance MCP tool handlers driven through UniversalToolExecutor
// ABOUTME: Covers the 9 endurance Phase 1-5 tools: exports, history, intervals, routes, streams, workouts
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Endurance Tool Handler Integration Tests
//!
//! Tests the 9 endurance MCP tools via the `UniversalToolExecutor`:
//! - `export_dossier`, `export_latest_snapshot` (Phase 1)
//! - `compute_training_history`, `get_training_history` (Phase 2)
//! - `export_intervals`, `export_routes`, `extract_activity_streams` (Phase 3)
//! - `list_workout_templates`, `prescribe_workout` (Phase 5)
//!
//! Coverage strategy:
//! - **DB-only tools** (`export_dossier`, `get_training_history`,
//!   `list_workout_templates`, `prescribe_workout`) get a real happy path
//!   against seeded fixtures.
//! - **Provider-gated tools** (`export_latest_snapshot`,
//!   `compute_training_history`, `export_intervals`, `export_routes`,
//!   `extract_activity_streams`) exercise the provider-auth-required path —
//!   the tool plumbing runs all the way through `fetch_activities_from_provider`
//!   and returns the documented `AppError::auth_invalid` when no OAuth token
//!   is connected. This validates the tool's auth gating, tenant gating, and
//!   argument parsing without needing a mock provider.
//! - Every tool also gets a no-tenant rejection test and an input-validation
//!   test (missing required arg or out-of-range value).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::{SportType, TenantId, UserPhysiologicalProfile};
use pierre_mcp_server::protocols::universal::{UniversalRequest, UniversalToolExecutor};
use pierre_mcp_server::protocols::ProtocolError;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

// ============================================================================
// Test setup
// ============================================================================

async fn create_endurance_test_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

/// Create a regular test user with their own tenant. Endurance tools only
/// require an authenticated user with a tenant — no admin role check.
async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("endurance_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(&executor.resources.database, &email).await?;
    let tenants = executor.resources.repos.tenants.get_all().await?;
    let user_tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have tenant"))?;
    Ok((user_id, user_tenant.id.to_string()))
}

fn make_request(
    tool: &str,
    params: Value,
    user_id: Uuid,
    tenant_id: Option<&str>,
) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: tenant_id.map(str::to_owned),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

fn assert_invalid_request(err: &ProtocolError, tool: &str) {
    match err {
        ProtocolError::InvalidRequest(_) => {}
        other => panic!("{tool}: expected InvalidRequest, got {other:?}"),
    }
}

/// Seed a `UserPhysiologicalProfile` for the user so `export_dossier` returns
/// a populated payload rather than a fully-null shape.
async fn seed_physiology(
    executor: &UniversalToolExecutor,
    tenant: &str,
    user_id: Uuid,
) -> Result<()> {
    let profile = UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(52.0),
        resting_hr: Some(50),
        max_hr: Some(190),
        lactate_threshold_percentage: Some(0.85),
        age: Some(34),
        weight: Some(72.0),
        fitness_level: FitnessLevel::Advanced,
        primary_sport: SportType::Run,
        training_experience_years: Some(10),
        ftp_watts: Some(280),
        threshold_pace_sec_per_km: Some(225.0),
        hr_zones: None,
        power_zones: None,
    };
    let tenant_id: TenantId = tenant.parse().expect("valid tenant id");
    executor
        .resources
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(tenant_id, user_id, &profile)
        .await?;
    Ok(())
}

// ============================================================================
// Tool registration sanity check
// ============================================================================

#[tokio::test]
async fn test_endurance_tools_registered() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let names: Vec<String> = executor
        .resources
        .tool_registry
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    for expected in [
        "export_dossier",
        "export_latest_snapshot",
        "compute_training_history",
        "get_training_history",
        "export_intervals",
        "export_routes",
        "extract_activity_streams",
        "list_workout_templates",
        "prescribe_workout",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "tool registry missing {expected}"
        );
    }
    Ok(())
}

// ============================================================================
// export_dossier — DB-only, real happy path
// ============================================================================

#[tokio::test]
async fn test_export_dossier_happy_path_empty() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "export_dossier",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success, "dossier should succeed: {:?}", resp.error);
    // Dossier shape: physiology / zones / goals / nutrition / equipment — fresh
    // user has all-null slots but the keys are still present.
    let result = resp.result.unwrap();
    assert!(result.is_object(), "dossier payload must be a JSON object");
    Ok(())
}

#[tokio::test]
async fn test_export_dossier_with_seeded_physiology() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    seed_physiology(&executor, &tenant, user_id).await?;

    let resp = executor
        .execute_tool(make_request(
            "export_dossier",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    // The seeded VO2 max must surface somewhere in the dossier. We don't pin
    // the exact path because the dossier composer may rename the slot — we
    // just verify the value made it through.
    let serialised = serde_json::to_string(&result)?;
    assert!(
        serialised.contains("52"),
        "seeded vo2_max=52 must appear in dossier payload"
    );
    Ok(())
}

#[tokio::test]
async fn test_export_dossier_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request("export_dossier", json!({}), user_id, None))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "export_dossier");
    Ok(())
}

// ============================================================================
// export_latest_snapshot — provider-gated
// ============================================================================

#[tokio::test]
async fn test_export_latest_snapshot_no_provider() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_latest_snapshot",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("user with no provider must surface auth-required");
    assert_invalid_request(&err, "export_latest_snapshot");
    Ok(())
}

#[tokio::test]
async fn test_export_latest_snapshot_clamps_window() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    // Out-of-range window is clamped, not rejected; the underlying provider
    // call still fails (no token) — we just confirm the clamp didn't panic
    // and the failure is the documented auth-required path.
    let err = executor
        .execute_tool(make_request(
            "export_latest_snapshot",
            json!({ "window": 9_999 }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("provider auth still required");
    assert_invalid_request(&err, "export_latest_snapshot");
    Ok(())
}

#[tokio::test]
async fn test_export_latest_snapshot_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_latest_snapshot",
            json!({}),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "export_latest_snapshot");
    Ok(())
}

// ============================================================================
// compute_training_history
// ============================================================================

#[tokio::test]
async fn test_compute_training_history_invalid_window() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "compute_training_history",
            json!({ "from": "2026-06-01", "to": "2026-01-01" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("from > to must be rejected");
    assert_invalid_request(&err, "compute_training_history");
    Ok(())
}

#[tokio::test]
async fn test_compute_training_history_no_provider() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "compute_training_history",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("user with no provider must surface auth-required");
    assert_invalid_request(&err, "compute_training_history");
    Ok(())
}

#[tokio::test]
async fn test_compute_training_history_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "compute_training_history",
            json!({}),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "compute_training_history");
    Ok(())
}

// ============================================================================
// get_training_history — DB-only, real happy path on empty table
// ============================================================================

#[tokio::test]
async fn test_get_training_history_empty() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_training_history",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "get_training_history should succeed on empty data: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert!(result["from"].as_str().is_some());
    assert!(result["to"].as_str().is_some());
    assert_eq!(
        result["days"]
            .as_array()
            .expect("days field must be array")
            .len(),
        0
    );
    Ok(())
}

#[tokio::test]
async fn test_get_training_history_invalid_date() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "get_training_history",
            json!({ "from": "not-a-date" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("invalid date format must be rejected");
    assert_invalid_request(&err, "get_training_history");
    Ok(())
}

#[tokio::test]
async fn test_get_training_history_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "get_training_history",
            json!({}),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "get_training_history");
    Ok(())
}

// ============================================================================
// export_intervals
// ============================================================================

#[tokio::test]
async fn test_export_intervals_missing_activity_id() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_intervals",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("missing activity_id must be rejected");
    assert_invalid_request(&err, "export_intervals");
    Ok(())
}

#[tokio::test]
async fn test_export_intervals_no_provider() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_intervals",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("user with no provider must surface auth-required");
    assert_invalid_request(&err, "export_intervals");
    Ok(())
}

#[tokio::test]
async fn test_export_intervals_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_intervals",
            json!({ "activity_id": "12345" }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "export_intervals");
    Ok(())
}

// ============================================================================
// export_routes
// ============================================================================

#[tokio::test]
async fn test_export_routes_missing_activity_id() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_routes",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("missing activity_id must be rejected");
    assert_invalid_request(&err, "export_routes");
    Ok(())
}

#[tokio::test]
async fn test_export_routes_no_provider() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_routes",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("user with no provider must surface auth-required");
    assert_invalid_request(&err, "export_routes");
    Ok(())
}

#[tokio::test]
async fn test_export_routes_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "export_routes",
            json!({ "activity_id": "12345" }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "export_routes");
    Ok(())
}

// ============================================================================
// extract_activity_streams
// ============================================================================

#[tokio::test]
async fn test_extract_activity_streams_missing_activity_id() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "extract_activity_streams",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("missing activity_id must be rejected");
    assert_invalid_request(&err, "extract_activity_streams");
    Ok(())
}

#[tokio::test]
async fn test_extract_activity_streams_no_provider() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "extract_activity_streams",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("user with no provider must surface auth-required");
    assert_invalid_request(&err, "extract_activity_streams");
    Ok(())
}

#[tokio::test]
async fn test_extract_activity_streams_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "extract_activity_streams",
            json!({ "activity_id": "12345" }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "extract_activity_streams");
    Ok(())
}

// ============================================================================
// list_workout_templates — pure read-only, no provider, no tenant required
// ============================================================================

#[tokio::test]
async fn test_list_workout_templates_happy_path() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "list_workout_templates",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "list_workout_templates must succeed: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    let count = result["count"].as_u64().expect("count field required");
    assert!(
        count >= 6,
        "expected at least 6 cornerstone templates, got {count}"
    );
    let templates = result["templates"].as_array().expect("templates array");
    assert_eq!(templates.len() as u64, count);
    Ok(())
}

#[tokio::test]
async fn test_list_workout_templates_includes_cornerstones() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "list_workout_templates",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    let result = resp.result.unwrap();
    let slugs: Vec<&str> = result["templates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["slug"].as_str())
        .collect();
    for cornerstone in [
        "long_run_z2",
        "threshold_4x8",
        "vo2_5x3",
        "recovery_30min",
        "tempo_progression",
        "sweet_spot_2x20",
    ] {
        assert!(
            slugs.iter().any(|s| s == &cornerstone),
            "cornerstone '{cornerstone}' missing from {slugs:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_list_workout_templates_idempotent() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let first = executor
        .execute_tool(make_request(
            "list_workout_templates",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    let second = executor
        .execute_tool(make_request(
            "list_workout_templates",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert_eq!(first.result, second.result, "templates must be stable");
    Ok(())
}

// ============================================================================
// prescribe_workout — DB writes, no real provider push in test
// ============================================================================

#[tokio::test]
async fn test_prescribe_workout_happy_path() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "prescribe_workout",
            json!({
                "template_slug": "long_run_z2",
                "date": "2026-06-15",
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "prescribe_workout must succeed: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert_eq!(result["template_slug"].as_str().unwrap(), "long_run_z2");
    assert_eq!(result["scheduled_for"].as_str().unwrap(), "2026-06-15");
    assert_eq!(result["status"].as_str().unwrap(), "queued");
    assert!(result["prescription_id"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_prescribe_workout_invalid_slug() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "prescribe_workout",
            json!({
                "template_slug": "not_a_real_template",
                "date": "2026-06-15",
            }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("unknown template slug must be rejected");
    // require_cornerstone returns AppError::not_found which the executor maps
    // to ProtocolError::InternalError. Either error variant proves the tool
    // rejected the bad slug instead of writing a phantom row.
    let msg = format!("{err:?}");
    assert!(
        msg.contains("not_a_real_template") || msg.contains("not found"),
        "expected slug-not-found error, got {msg}"
    );
    Ok(())
}

#[tokio::test]
async fn test_prescribe_workout_missing_date() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "prescribe_workout",
            json!({ "template_slug": "long_run_z2" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("missing date must be rejected");
    assert_invalid_request(&err, "prescribe_workout");
    Ok(())
}

#[tokio::test]
async fn test_prescribe_workout_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "prescribe_workout",
            json!({
                "template_slug": "long_run_z2",
                "date": "2026-06-15",
            }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "prescribe_workout");
    Ok(())
}
