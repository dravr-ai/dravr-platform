// ABOUTME: Integration tests for MCP tool quota enforcement
// ABOUTME: Validates pre-execution checks, counter increments, and `llm_usage` recording
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::redundant_closure_for_method_calls)]

mod common;
mod integration;

use anyhow::Result;
use chrono::{Datelike, Duration, Utc};
use integration::{IntegrationTestServer, McpTestClient};
use pierre_database::database::repositories::UsageCounterRepository;
use serde_json::json;

// ============================================================================
// TEST HELPERS
// ============================================================================

/// Create a test server, user, and MCP client, returning all three plus tenant/user IDs
async fn setup_quota_test(
    email: &str,
) -> Result<(IntegrationTestServer, McpTestClient, String, String)> {
    let mut server = IntegrationTestServer::new().await?;
    server.start().await?;
    let (user_id, tenant_id, jwt_token) = server.create_test_user_with_tenant(email).await?;
    let client = McpTestClient::new(&server.base_url(), &jwt_token);
    Ok((server, client, tenant_id.to_string(), user_id.to_string()))
}

/// Compute the current daily period string (YYYY-MM-DD)
fn current_daily_period() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// Compute the current weekly period string (most recent Sunday, YYYY-MM-DD)
fn current_weekly_period() -> String {
    let now = Utc::now();
    let days_since_sunday = now.weekday().num_days_from_sunday();
    let sunday = now - Duration::days(i64::from(days_since_sunday));
    sunday.format("%Y-%m-%d").to_string()
}

/// Make a tool call and return the raw `JSON-RPC` response (includes error field if any)
async fn call_tool_raw_response(
    client: &McpTestClient,
    tool_name: &str,
) -> Result<serde_json::Value> {
    client
        .send_request_raw(
            "tools/call",
            Some(json!({
                "name": tool_name,
                "arguments": { "provider": "synthetic", "limit": 1 }
            })),
        )
        .await
}

/// Make a `get_activities` call with a specific mode (summary or detailed)
async fn call_get_activities_with_mode(
    client: &McpTestClient,
    mode: &str,
) -> Result<serde_json::Value> {
    client
        .send_request_raw(
            "tools/call",
            Some(json!({
                "name": "get_activities",
                "arguments": { "provider": "synthetic", "limit": 1, "mode": mode }
            })),
        )
        .await
}

// ============================================================================
// ASY-588: Counter increment tests
// ============================================================================

/// Verify that a successful MCP tool call increments the `daily_tool_calls` counter
#[tokio::test]
async fn test_mcp_tool_increments_daily_counter() -> Result<()> {
    let (server, client, tenant_id, user_id) = setup_quota_test("quota-daily@test.local").await?;

    // Get counter before
    let before = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_tool_calls",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(before.value, 0, "Counter should start at zero");

    // Make a successful tool call
    let response = call_tool_raw_response(&client, "get_activities").await?;
    assert!(
        response.get("result").is_some(),
        "Tool call should succeed: {response}"
    );

    // Check counter was incremented
    let after = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_tool_calls",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(after.value, 1, "Daily counter should be incremented to 1");

    // Also check weekly counter
    let weekly = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "weekly_tool_calls",
            &current_weekly_period(),
        )
        .await?;
    assert_eq!(weekly.value, 1, "Weekly counter should be incremented to 1");

    println!("test_mcp_tool_increments_daily_counter passed");
    Ok(())
}

// ============================================================================
// ASY-587: Quota blocking tests
// ============================================================================

/// Verify that exceeding the daily tool call limit returns `JSON-RPC` error -32029
#[tokio::test]
async fn test_mcp_tool_blocked_at_daily_limit() -> Result<()> {
    let (server, client, tenant_id, user_id) =
        setup_quota_test("quota-block-daily@test.local").await?;

    // Default daily_tool_calls limit is 100, burst multiplier 1.5x = 150 hard limit.
    // Pre-seed counter to 150 so the next check_limit returns allowed=false.
    server
        .resources()
        .database
        .increment_counter(
            &tenant_id,
            &user_id,
            "daily_tool_calls",
            &current_daily_period(),
            150,
        )
        .await?;

    // Tool call should be blocked
    let response = call_tool_raw_response(&client, "get_activities").await?;
    let error = response.get("error").expect("Should have error field");

    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32029),
        "Error code should be -32029 (rate limit exceeded)"
    );
    assert_eq!(
        error
            .get("data")
            .and_then(|d| d.get("limit_type"))
            .and_then(|l| l.as_str()),
        Some("daily_tool_calls"),
        "Error data should contain limit_type"
    );

    println!("test_mcp_tool_blocked_at_daily_limit passed");
    Ok(())
}

/// Verify that exceeding the weekly tool call limit returns `JSON-RPC` error -32029
#[tokio::test]
async fn test_mcp_tool_blocked_at_weekly_limit() -> Result<()> {
    let (server, client, tenant_id, user_id) =
        setup_quota_test("quota-block-weekly@test.local").await?;

    // Default weekly_tool_calls limit is 500, burst multiplier 1.5x = 750 hard limit.
    // Pre-seed weekly counter to 750 so the next check_limit returns allowed=false.
    server
        .resources()
        .database
        .increment_counter(
            &tenant_id,
            &user_id,
            "weekly_tool_calls",
            &current_weekly_period(),
            750,
        )
        .await?;

    // Tool call should be blocked by weekly limit
    let response = call_tool_raw_response(&client, "get_activities").await?;
    let error = response.get("error").expect("Should have error field");

    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32029),
        "Error code should be -32029"
    );
    assert_eq!(
        error
            .get("data")
            .and_then(|d| d.get("limit_type"))
            .and_then(|l| l.as_str()),
        Some("weekly_tool_calls"),
        "Error data should contain weekly_tool_calls limit_type"
    );

    println!("test_mcp_tool_blocked_at_weekly_limit passed");
    Ok(())
}

// ============================================================================
// Shared budget test
// ============================================================================

/// Verify that pre-seeded counters (simulating chat usage) affect MCP tool calls
#[tokio::test]
async fn test_shared_budget_chat_and_mcp() -> Result<()> {
    let (server, client, tenant_id, user_id) = setup_quota_test("shared-budget@test.local").await?;

    // Simulate chat usage that brought the daily counter near the hard limit (150).
    // Pre-seed to 149 — one more MCP call should succeed, then the next should be blocked.
    server
        .resources()
        .database
        .increment_counter(
            &tenant_id,
            &user_id,
            "daily_tool_calls",
            &current_daily_period(),
            149,
        )
        .await?;

    // First MCP call should succeed (counter goes to 150 after execution)
    let response1 = call_tool_raw_response(&client, "get_activities").await?;
    assert!(
        response1.get("result").is_some(),
        "First call should succeed with counter at 149"
    );

    // Second MCP call should be blocked (counter is now 150, which equals hard limit)
    let response2 = call_tool_raw_response(&client, "get_activities").await?;
    let error = response2
        .get("error")
        .expect("Second call should be blocked");
    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32029),
        "Second call should return rate limit error"
    );

    println!("test_shared_budget_chat_and_mcp passed");
    Ok(())
}

// ============================================================================
// ASY-588: Failed tool does not increment
// ============================================================================

/// Verify that a failed tool execution does not increment usage counters
#[tokio::test]
async fn test_failed_tool_does_not_increment() -> Result<()> {
    let (server, client, tenant_id, user_id) = setup_quota_test("failed-tool@test.local").await?;

    // Call a nonexistent tool — produces a JSON-RPC error (not a tool result),
    // so handle_tool_execution_direct returns before the success path that increments counters.
    let response = client
        .send_request_raw(
            "tools/call",
            Some(json!({
                "name": "nonexistent_tool_that_does_not_exist",
                "arguments": {}
            })),
        )
        .await?;

    // Verify we got an error response (tool not found)
    assert!(
        response.get("error").is_some(),
        "Nonexistent tool should return JSON-RPC error: {response}"
    );

    // Counter should remain at 0 — failed tool calls do not count against quota
    let counter = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_tool_calls",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(counter.value, 0, "Counter must be 0 after failed tool call");

    println!("test_failed_tool_does_not_increment passed");
    Ok(())
}

// ============================================================================
// ASY-589: LLM usage record creation
// ============================================================================

/// Verify that a successful MCP tool call creates an `llm_usage` record with `call_type` `mcp_tool`
#[tokio::test]
async fn test_mcp_tool_creates_usage_record() -> Result<()> {
    let (server, client, tenant_id, user_id) = setup_quota_test("usage-record@test.local").await?;

    // Make a successful tool call
    let response = call_tool_raw_response(&client, "get_activities").await?;
    assert!(
        response.get("result").is_some(),
        "Tool call should succeed: {response}"
    );

    // Query llm_usage table directly to verify the record was created
    let pool = server
        .resources()
        .database
        .sqlite_pool()
        .expect("SQLite pool required");

    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM llm_usage WHERE tenant_id = ?1 AND user_id = ?2 AND call_type = 'mcp_tool'",
    )
    .bind(&tenant_id)
    .bind(&user_id)
    .fetch_one(pool)
    .await?;

    assert_eq!(
        row.0, 1,
        "Should have exactly 1 llm_usage record with call_type=mcp_tool"
    );

    // Verify the record fields
    let record: (String, String, i64, i64) = sqlx::query_as(
        "SELECT provider, model, tool_calls_count, total_tokens FROM llm_usage WHERE tenant_id = ?1 AND user_id = ?2 AND call_type = 'mcp_tool' LIMIT 1",
    )
    .bind(&tenant_id)
    .bind(&user_id)
    .fetch_one(pool)
    .await?;

    assert_eq!(record.0, "direct", "Provider should be 'direct'");
    assert_eq!(record.1, "get_activities", "Model should be the tool name");
    assert_eq!(record.2, 1, "tool_calls_count should be 1");
    assert_eq!(
        record.3, 0,
        "total_tokens should be 0 for direct tool calls"
    );

    println!("test_mcp_tool_creates_usage_record passed");
    Ok(())
}

// ============================================================================
// ASY-590: Quota error response format
// ============================================================================

/// Verify the `JSON-RPC` error response contains all required fields
#[tokio::test]
async fn test_quota_error_response_format() -> Result<()> {
    let (server, client, tenant_id, user_id) = setup_quota_test("error-format@test.local").await?;

    // Pre-seed to exceed hard limit
    server
        .resources()
        .database
        .increment_counter(
            &tenant_id,
            &user_id,
            "daily_tool_calls",
            &current_daily_period(),
            150,
        )
        .await?;

    let response = call_tool_raw_response(&client, "get_activities").await?;

    // Verify JSON-RPC envelope
    assert_eq!(
        response.get("jsonrpc").and_then(|v| v.as_str()),
        Some("2.0"),
        "Should have jsonrpc: 2.0"
    );
    assert!(response.get("id").is_some(), "Should have request id");

    // Verify error object
    let error = response.get("error").expect("Should have error field");
    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32029),
        "Error code should be -32029"
    );
    assert_eq!(
        error.get("message").and_then(|m| m.as_str()),
        Some("Rate limit exceeded"),
        "Error message should be 'Rate limit exceeded'"
    );

    // Verify error data fields
    let data = error.get("data").expect("Error should have data field");
    assert_eq!(
        data.get("limit_type").and_then(|l| l.as_str()),
        Some("daily_tool_calls"),
        "Data should contain limit_type"
    );
    assert!(
        data.get("current").is_some(),
        "Data should contain current count"
    );
    assert!(
        data.get("limit").is_some(),
        "Data should contain limit value"
    );
    assert!(
        data.get("resets_at").and_then(|r| r.as_str()).is_some(),
        "Data should contain resets_at timestamp"
    );

    println!("test_quota_error_response_format passed");
    Ok(())
}

// ============================================================================
// Activity access quota tests (DRAVR-428)
// ============================================================================

/// Verify that a summary mode `get_activities` increments `daily_activity_summary` counter
#[tokio::test]
async fn test_activity_summary_increments_counter() -> Result<()> {
    let (server, client, tenant_id, user_id) =
        setup_quota_test("activity-summary@test.local").await?;

    // Counter should start at zero
    let before = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_activity_summary",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(before.value, 0, "Summary counter should start at zero");

    // Call get_activities in summary mode (default)
    let response = call_get_activities_with_mode(&client, "summary").await?;
    assert!(
        response.get("result").is_some(),
        "Summary tool call should succeed: {response}"
    );

    // Daily summary counter should be incremented
    let after = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_activity_summary",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(after.value, 1, "Daily activity summary counter should be 1");

    // Weekly summary counter should also be incremented
    let weekly = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "weekly_activity_summary",
            &current_weekly_period(),
        )
        .await?;
    assert_eq!(
        weekly.value, 1,
        "Weekly activity summary counter should be 1"
    );

    // Detailed counters should remain at zero
    let detailed_daily = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_activity_detailed",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(
        detailed_daily.value, 0,
        "Detailed counter should not increment for summary mode"
    );

    println!("test_activity_summary_increments_counter passed");
    Ok(())
}

/// Verify that a detailed mode `get_activities` increments `daily_activity_detailed` counter
#[tokio::test]
async fn test_activity_detailed_increments_counter() -> Result<()> {
    let (server, client, tenant_id, user_id) =
        setup_quota_test("activity-detailed@test.local").await?;

    // Call get_activities in detailed mode
    let response = call_get_activities_with_mode(&client, "detailed").await?;
    assert!(
        response.get("result").is_some(),
        "Detailed tool call should succeed: {response}"
    );

    // Daily detailed counter should be incremented
    let detailed_daily = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_activity_detailed",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(
        detailed_daily.value, 1,
        "Daily activity detailed counter should be 1"
    );

    // Weekly detailed counter should also be incremented
    let detailed_weekly = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "weekly_activity_detailed",
            &current_weekly_period(),
        )
        .await?;
    assert_eq!(
        detailed_weekly.value, 1,
        "Weekly activity detailed counter should be 1"
    );

    // Summary counters should remain at zero
    let summary_daily = server
        .resources()
        .database
        .get_counter(
            &tenant_id,
            &user_id,
            "daily_activity_summary",
            &current_daily_period(),
        )
        .await?;
    assert_eq!(
        summary_daily.value, 0,
        "Summary counter should not increment for detailed mode"
    );

    println!("test_activity_detailed_increments_counter passed");
    Ok(())
}

/// Verify that exceeding the daily detailed activity limit blocks the request
#[tokio::test]
async fn test_activity_detailed_blocked_at_daily_limit() -> Result<()> {
    let (server, client, tenant_id, user_id) =
        setup_quota_test("activity-block@test.local").await?;

    // Default daily_activity_detailed limit is 20, burst 1.5x = 30 hard limit.
    // Pre-seed counter to 30 so next check returns allowed=false.
    server
        .resources()
        .database
        .increment_counter(
            &tenant_id,
            &user_id,
            "daily_activity_detailed",
            &current_daily_period(),
            30,
        )
        .await?;

    // Detailed request should be blocked
    let response = call_get_activities_with_mode(&client, "detailed").await?;
    let error = response
        .get("error")
        .expect("Detailed call should be blocked");
    assert_eq!(
        error.get("code").and_then(|c| c.as_i64()),
        Some(-32029),
        "Error code should be -32029 (rate limit exceeded)"
    );
    assert_eq!(
        error
            .get("data")
            .and_then(|d| d.get("limit_type"))
            .and_then(|l| l.as_str()),
        Some("daily_activity_detailed"),
        "Error should reference the activity detailed limit"
    );

    // Summary mode should still work (different counter)
    let summary_response = call_get_activities_with_mode(&client, "summary").await?;
    assert!(
        summary_response.get("result").is_some(),
        "Summary call should succeed even when detailed is blocked: {summary_response}"
    );

    println!("test_activity_detailed_blocked_at_daily_limit passed");
    Ok(())
}
