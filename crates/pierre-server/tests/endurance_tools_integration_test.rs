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
use pierre_core::models::{ConnectionType, SportType, TenantId, UserPhysiologicalProfile};
use pierre_tool_runtime::protocols::ProtocolError;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
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
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let user_tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have tenant"))?;
    Ok((user_id, user_tenant.id.to_string()))
}

/// Same as [`create_test_user`] but with a provider connection registered.
///
/// Argument-validation tests need this now. The dispatch chokepoint refuses a
/// `REQUIRES_PROVIDER` tool before its body parses anything, so a providerless
/// fixture can only ever prove the refusal — never that a reversed date range
/// is rejected. Connecting a provider restores what these tests were written to
/// check.
async fn create_connected_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let (user_id, tenant) = create_test_user(executor).await?;
    executor
        .resources
        .repos()
        .provider_connections
        .register_connection(
            user_id,
            TenantId::parse_str(&tenant)?,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await?;
    Ok((user_id, tenant))
}

/// A providerless call is refused **in band** rather than by abandoning the turn.
///
/// These tools declared `REQUIRES_PROVIDER` long before anything read it, so the
/// refusal used to surface from deep in the tool body as a `ProtocolError`,
/// which kills the turn. The dispatch chokepoint refuses first and returns the
/// same `UniversalResponse` the provider resolvers mint, carrying
/// `auth_required_provider` — the signal `auth_recovery` turns into a
/// hosted-login prompt, so the coach can offer to fix it instead of the turn
/// simply dying.
fn assert_provider_refusal(resp: &UniversalResponse, tool: &str) {
    assert!(!resp.success, "{tool}: providerless call must not succeed");
    let provider = resp
        .metadata
        .as_ref()
        .and_then(|m| m.get("auth_required_provider"))
        .and_then(serde_json::Value::as_str);
    assert_eq!(
        provider,
        Some("sciotte"),
        "{tool}: the refusal must carry the recovery metadata, got {:?}",
        resp.metadata
    );
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

/// Accept both `ProtocolError` variants the executor can produce when a tool
/// rejects a call: `InvalidParameters` for missing/malformed fields (mapped
/// from `AppError::InvalidInput`) and `InvalidRequest` for the tenant-gate
/// rejection. Either proves the tool refused to run.
fn assert_invalid_request(err: &ProtocolError, tool: &str) {
    match err {
        ProtocolError::InvalidParameters(_) | ProtocolError::InvalidRequest(_) => {}
        other => panic!("{tool}: expected InvalidParameters or InvalidRequest, got {other:?}"),
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
    let tenant_id = TenantId::parse_str(tenant).expect("valid tenant id");
    executor
        .resources
        .repos()
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
        .tool_registry()
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
        "withdraw_prescribed_workout",
        "push_training_plan",
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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;
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
    // LT1/LT2 threshold estimation is wired into the dossier: the seeded FTP
    // (280W) yields LT2 power = 280 and LT1 power = 0.75 * 280 = 210 (Coggan),
    // surfaced under `threshold_estimate`.
    let estimate = &result["threshold_estimate"];
    assert_eq!(
        estimate["lt2_power_watts"].as_f64(),
        Some(280.0),
        "LT2 power should equal the seeded FTP"
    );
    assert_eq!(
        estimate["lt1_power_watts"].as_f64(),
        Some(210.0),
        "LT1 power should be 0.75 * FTP (Coggan)"
    );
    Ok(())
}

#[tokio::test]
async fn test_export_dossier_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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

    let resp = executor
        .execute_tool(make_request(
            "export_latest_snapshot",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect("the refusal is in band; the turn must not be abandoned");
    assert_provider_refusal(&resp, "export_latest_snapshot");
    Ok(())
}

#[tokio::test]
async fn test_export_latest_snapshot_clamps_window() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_connected_test_user(&executor).await?;
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
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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

    let resp = executor
        .execute_tool(make_request(
            "compute_training_history",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect("the refusal is in band; the turn must not be abandoned");
    assert_provider_refusal(&resp, "compute_training_history");
    Ok(())
}

#[tokio::test]
async fn test_compute_training_history_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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

    // This is the tool the endurance coach prompt says to call first, and it
    // used to answer with bare ctl/atl/tsb floats. The interpretation key ships
    // even on an empty window, because the coach reads it to know what the
    // numbers mean before it has any (registre#199).
    let method = result["interpretation"]["method"]
        .as_str()
        .expect("get_training_history must carry the method that produced its numbers");
    assert!(
        method.contains("CTL - ATL"),
        "the formula must reach the coach: {method}"
    );
    assert!(
        result["interpretation"]["tsb"]
            .as_str()
            .is_some_and(|t| t.contains("interpret via tsb_pct_of_ctl")),
        "and the instruction not to read the raw number: {}",
        result["interpretation"]
    );
    Ok(())
}

#[tokio::test]
async fn test_get_training_history_invalid_date() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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

    let resp = executor
        .execute_tool(make_request(
            "export_intervals",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect("the refusal is in band; the turn must not be abandoned");
    assert_provider_refusal(&resp, "export_intervals");
    Ok(())
}

#[tokio::test]
async fn test_export_intervals_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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

    let resp = executor
        .execute_tool(make_request(
            "export_routes",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect("the refusal is in band; the turn must not be abandoned");
    assert_provider_refusal(&resp, "export_routes");
    Ok(())
}

#[tokio::test]
async fn test_export_routes_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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

    let resp = executor
        .execute_tool(make_request(
            "extract_activity_streams",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect("the refusal is in band; the turn must not be abandoned");
    assert_provider_refusal(&resp, "extract_activity_streams");
    Ok(())
}

#[tokio::test]
async fn test_extract_activity_streams_rejects_no_tenant() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
// prescribe_workout — argument gating; the push itself is `workout_push_test`
// ============================================================================

#[tokio::test]
async fn test_prescribe_workout_requires_a_writable_calendar() -> Result<()> {
    // The fixture user is Strava-connected, which clears the dispatch
    // chokepoint but gives them no calendar to write to. Intervals.icu is the
    // only provider with a planned-workout write surface, so the tool must say
    // so rather than record a prescription that reaches no one — the exact
    // failure carnet#100 was filed for. The real push, against a stubbed
    // Intervals.icu, is covered in `workout_push_test.rs`.
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
        .await;
    let rendered = match resp {
        Ok(response) => {
            assert!(
                !response.success,
                "a prescription with no writable calendar must not report success"
            );
            format!("{:?}", response.error)
        }
        Err(err) => format!("{err:?}"),
    };
    assert!(
        rendered.to_lowercase().contains("intervals"),
        "the refusal must name the account the athlete has to connect; got: {rendered}"
    );
    Ok(())
}

#[tokio::test]
async fn test_prescribe_workout_invalid_slug() -> Result<()> {
    let executor = create_endurance_test_executor().await?;
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
    // A slug is resolved against the cornerstones and then this athlete's own
    // saved sessions; matching neither is AppError::not_found, which the
    // executor maps to ProtocolError::InternalError. Either error variant
    // proves the tool rejected the bad slug instead of writing a phantom row.
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
    let (user_id, tenant) = create_connected_test_user(&executor).await?;

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
    let (user_id, _tenant) = create_connected_test_user(&executor).await?;

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
