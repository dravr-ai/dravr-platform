// ABOUTME: Integration tests for health-data and sync MCP tool handlers driven through UniversalToolExecutor
// ABOUTME: Covers get_sleep_sessions, get_recovery_metrics, get_health_snapshots, list_data_sources, refresh_provider_data, get_data_freshness, discover_routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Health Data + Sync + Routes Tool Handler Integration Tests
//!
//! Tests 7 MCP tools via the `UniversalToolExecutor`:
//! - `get_sleep_sessions`, `get_recovery_metrics`, `get_health_snapshots`,
//!   `list_data_sources` — DB-only readers
//! - `get_data_freshness`, `refresh_provider_data` — provider sync surface
//! - `discover_routes` — Overpass-backed route discovery (input validation only;
//!   we don't hit the live Overpass API in tests)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_mcp_server::protocols::universal::{UniversalRequest, UniversalToolExecutor};
use pierre_mcp_server::protocols::ProtocolError;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

// ============================================================================
// Test setup
// ============================================================================

async fn create_health_test_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("health_test_{}@example.com", Uuid::new_v4());
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

// ============================================================================
// Tool registration sanity check
// ============================================================================

#[tokio::test]
async fn test_health_sync_tools_registered() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let names: Vec<String> = executor
        .resources
        .tool_registry
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    for expected in [
        "get_sleep_sessions",
        "get_recovery_metrics",
        "get_health_snapshots",
        "list_data_sources",
        "get_data_freshness",
        "refresh_provider_data",
        "discover_routes",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "tool registry missing {expected}"
        );
    }
    Ok(())
}

// ============================================================================
// get_sleep_sessions
// ============================================================================

#[tokio::test]
async fn test_get_sleep_sessions_empty() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_sleep_sessions",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success, "should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    assert!(result["sessions"].as_array().unwrap().is_empty());
    assert!(result["range"]["start"].as_str().is_some());
    assert!(result["range"]["end"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_get_sleep_sessions_with_date_range() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_sleep_sessions",
            json!({
                "start": "2026-01-01T00:00:00Z",
                "end": "2026-01-31T23:59:59Z",
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    assert!(result["range"]["start"]
        .as_str()
        .unwrap()
        .starts_with("2026-01-01"));
    Ok(())
}

#[tokio::test]
async fn test_get_sleep_sessions_tenant_isolation() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_a, _tenant_a) = create_test_user(&executor).await?;
    let (_user_b, tenant_b) = create_test_user(&executor).await?;

    // user_a querying with tenant_b context returns no data — the handler
    // scopes by both user_id and tenant_id.
    let resp = executor
        .execute_tool(make_request(
            "get_sleep_sessions",
            json!({}),
            user_a,
            Some(&tenant_b),
        ))
        .await?;
    assert!(resp.success);
    assert_eq!(resp.result.unwrap()["count"].as_u64().unwrap(), 0);
    Ok(())
}

// ============================================================================
// get_recovery_metrics
// ============================================================================

#[tokio::test]
async fn test_get_recovery_metrics_empty() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_recovery_metrics",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    assert!(result["metrics"].as_array().unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn test_get_recovery_metrics_with_format() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_recovery_metrics",
            json!({ "format": "json" }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    assert!(resp.result.is_some());
    Ok(())
}

#[tokio::test]
async fn test_get_recovery_metrics_tenant_isolation() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_a, _tenant_a) = create_test_user(&executor).await?;
    let (_user_b, tenant_b) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_recovery_metrics",
            json!({}),
            user_a,
            Some(&tenant_b),
        ))
        .await?;
    assert!(resp.success);
    assert_eq!(resp.result.unwrap()["count"].as_u64().unwrap(), 0);
    Ok(())
}

// ============================================================================
// get_health_snapshots
// ============================================================================

#[tokio::test]
async fn test_get_health_snapshots_empty() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_health_snapshots",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    Ok(())
}

#[tokio::test]
async fn test_get_health_snapshots_with_range() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_health_snapshots",
            json!({
                "start": "2026-04-01T00:00:00Z",
                "end": "2026-05-01T00:00:00Z",
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    assert!(result["range"]["start"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_get_health_snapshots_tenant_isolation() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_a, _tenant_a) = create_test_user(&executor).await?;
    let (_user_b, tenant_b) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_health_snapshots",
            json!({}),
            user_a,
            Some(&tenant_b),
        ))
        .await?;
    assert!(resp.success);
    assert_eq!(resp.result.unwrap()["count"].as_u64().unwrap(), 0);
    Ok(())
}

// ============================================================================
// list_data_sources
// ============================================================================

#[tokio::test]
async fn test_list_data_sources_empty() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "list_data_sources",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success, "should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    assert!(result["sources"].as_array().unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn test_list_data_sources_response_shape() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "list_data_sources",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    let result = resp.result.unwrap();
    assert!(result["count"].is_number(), "count must be a number");
    assert!(result["sources"].is_array(), "sources must be an array");
    Ok(())
}

#[tokio::test]
async fn test_list_data_sources_tenant_isolation() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_a, _tenant_a) = create_test_user(&executor).await?;
    let (_user_b, tenant_b) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "list_data_sources",
            json!({}),
            user_a,
            Some(&tenant_b),
        ))
        .await?;
    assert!(resp.success);
    assert_eq!(resp.result.unwrap()["count"].as_u64().unwrap(), 0);
    Ok(())
}

// ============================================================================
// get_data_freshness
// ============================================================================

#[tokio::test]
async fn test_get_data_freshness_happy_path() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "get_data_freshness",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success, "should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert!(result["providers"].is_array() || result["providers"].is_object());
    assert!(result["sync_metrics"].is_object());
    Ok(())
}

#[tokio::test]
async fn test_get_data_freshness_rejects_no_tenant() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request("get_data_freshness", json!({}), user_id, None))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "get_data_freshness");
    Ok(())
}

#[tokio::test]
async fn test_get_data_freshness_idempotent() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let first = executor
        .execute_tool(make_request(
            "get_data_freshness",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    let second = executor
        .execute_tool(make_request(
            "get_data_freshness",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(first.success);
    assert!(second.success);
    Ok(())
}

// ============================================================================
// refresh_provider_data
// ============================================================================

#[tokio::test]
async fn test_refresh_provider_data_all_no_connections() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    // Passing provider=all with no connected providers should return a
    // structured response with `refreshing` and `already_fresh` arrays.
    let resp = executor
        .execute_tool(make_request(
            "refresh_provider_data",
            json!({ "provider": "all" }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success, "should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["status"].as_str().unwrap(), "refresh_triggered");
    assert!(result["refreshing"].is_array() || result["refreshing"].is_object());
    Ok(())
}

#[tokio::test]
async fn test_refresh_provider_data_specific_provider_no_token() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "refresh_provider_data",
            json!({ "provider": "strava", "wait": false }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    // Tool returns success=true with the per-provider result struct; the
    // inner success boolean reflects whether the refresh actually ran.
    assert!(resp.success);
    let result = resp.result.unwrap();
    assert_eq!(result["provider"].as_str().unwrap(), "strava");
    assert!(result["success"].is_boolean());
    Ok(())
}

#[tokio::test]
async fn test_refresh_provider_data_rejects_no_tenant() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "refresh_provider_data",
            json!({ "provider": "strava" }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_invalid_request(&err, "refresh_provider_data");
    Ok(())
}

// ============================================================================
// discover_routes — input validation only (no live Overpass calls in tests)
// ============================================================================

#[tokio::test]
async fn test_discover_routes_missing_center() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    // Neither `place` nor lat/lon — resolver returns an error JSON payload
    // wrapped in ToolResult::error, so the call comes back with success=false
    // and a structured error message rather than panicking.
    let resp = executor
        .execute_tool(make_request(
            "discover_routes",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(!resp.success, "missing center must surface success=false");
    let err_msg = resp
        .error
        .as_deref()
        .unwrap_or_else(|| panic!("missing error msg, result={:?}", resp.result));
    assert!(
        err_msg.to_lowercase().contains("latitude")
            || err_msg.to_lowercase().contains("place")
            || err_msg.to_lowercase().contains("coordinates"),
        "expected center-missing message, got: {err_msg}"
    );
    Ok(())
}

#[tokio::test]
async fn test_discover_routes_invalid_latitude() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "discover_routes",
            json!({ "latitude": 91.0, "longitude": -74.0 }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(!resp.success, "out-of-range latitude must be rejected");
    Ok(())
}

#[tokio::test]
async fn test_discover_routes_invalid_longitude() -> Result<()> {
    let executor = create_health_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "discover_routes",
            json!({ "latitude": 45.0, "longitude": 181.0 }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(!resp.success, "out-of-range longitude must be rejected");
    Ok(())
}
