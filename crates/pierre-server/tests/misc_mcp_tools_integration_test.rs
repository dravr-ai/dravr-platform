// ABOUTME: Integration tests for misc MCP tool handlers driven through UniversalToolExecutor
// ABOUTME: Covers coach_note_add, coach_followup_schedule, remember_fact, recall_user_memory, verify_claim, analyze_weather_impact
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Misc Tool Handler Integration Tests
//!
//! Tests the 6 remaining MCP tools that previously had only schema/registry
//! coverage:
//! - `coach_note_add`, `coach_followup_schedule` — coach-authored memory writes
//! - `remember_fact`, `recall_user_memory` — Letta-style active memory
//! - `verify_claim` — Tier 5.5 bullshit detector
//! - `analyze_weather_impact` — weather-correlation analytics (provider-gated)

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

async fn create_misc_test_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("misc_test_{}@example.com", Uuid::new_v4());
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

/// Memory and verification tools call `context.require_tenant()` which surfaces
/// an `AppError` with an authorization-class code that the executor maps to
/// `ProtocolError::InternalError` (not `InvalidRequest`). Either is acceptable
/// for "the tool refused to run without a tenant" — both prove the gate fires.
fn assert_tool_errored(err: &ProtocolError, tool: &str) {
    match err {
        ProtocolError::InvalidRequest(_) | ProtocolError::InternalError(_) => {}
        other => panic!("{tool}: expected error variant, got {other:?}"),
    }
}

/// Seed a real coach so coach-note / coach-followup tests have a valid
/// `coach_id` for the FK constraint to be satisfied.
async fn seed_coach(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
    title: &str,
) -> Result<String> {
    let resp = executor
        .execute_tool(make_request(
            "create_coach",
            json!({
                "title": title,
                "system_prompt": "test coach system prompt",
                "category": "training",
            }),
            user_id,
            Some(tenant_id),
        ))
        .await?;
    assert!(resp.success, "seed_coach must succeed: {:?}", resp.error);
    Ok(resp.result.unwrap()["id"]
        .as_str()
        .expect("created coach must have id")
        .to_owned())
}

// ============================================================================
// Registration sanity
// ============================================================================

#[tokio::test]
async fn test_misc_tools_registered() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let names: Vec<String> = executor
        .resources
        .tool_registry
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    for expected in [
        "coach_note_add",
        "coach_followup_schedule",
        "remember_fact",
        "recall_user_memory",
        "verify_claim",
        "analyze_weather_impact",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "tool registry missing {expected}"
        );
    }
    Ok(())
}

// ============================================================================
// coach_note_add
// ============================================================================

#[tokio::test]
async fn test_coach_note_add_happy_path() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let coach_id = seed_coach(&executor, user_id, &tenant, "Note Coach").await?;

    let resp = executor
        .execute_tool(make_request(
            "coach_note_add",
            json!({
                "content": "User prefers no scientific jargon when discussing zones.",
                "coach_id": coach_id,
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "coach_note_add should succeed: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert!(result["note_id"].is_string() || result["note_id"].is_number());
    assert!(result["created_at"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_coach_note_add_empty_content() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "coach_note_add",
            json!({
                "content": "   ",
                "coach_id": "test-coach-1",
            }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("empty content must be rejected");
    assert_invalid_request(&err, "coach_note_add");
    Ok(())
}

#[tokio::test]
async fn test_coach_note_add_missing_coach_id() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "coach_note_add",
            json!({ "content": "without coach_id" }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("missing coach_id must be rejected");
    assert_invalid_request(&err, "coach_note_add");
    Ok(())
}

#[tokio::test]
async fn test_coach_note_add_rejects_no_tenant() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "coach_note_add",
            json!({
                "content": "x",
                "coach_id": "test-coach-1",
            }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_tool_errored(&err, "coach_note_add");
    Ok(())
}

// ============================================================================
// coach_followup_schedule
// ============================================================================

#[tokio::test]
async fn test_coach_followup_schedule_happy_path() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;
    let coach_id = seed_coach(&executor, user_id, &tenant, "Followup Coach").await?;

    let resp = executor
        .execute_tool(make_request(
            "coach_followup_schedule",
            json!({
                "content": "check on Achilles pain after long run",
                "coach_id": coach_id,
                "due_at": "2026-06-15T10:00:00Z",
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "coach_followup_schedule should succeed: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert_eq!(result["status"].as_str().unwrap(), "pending");
    assert!(result["due_at"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_coach_followup_schedule_invalid_due_at() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "coach_followup_schedule",
            json!({
                "content": "x",
                "coach_id": "test-coach-1",
                "due_at": "not-a-real-date",
            }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("invalid due_at must be rejected");
    assert_invalid_request(&err, "coach_followup_schedule");
    Ok(())
}

#[tokio::test]
async fn test_coach_followup_schedule_rejects_no_tenant() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "coach_followup_schedule",
            json!({
                "content": "x",
                "coach_id": "test-coach-1",
            }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_tool_errored(&err, "coach_followup_schedule");
    Ok(())
}

// ============================================================================
// remember_fact
// ============================================================================

#[tokio::test]
async fn test_remember_fact_happy_path() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "remember_fact",
            json!({
                "kind": "preference",
                "subject": "you",
                "predicate": "prefers",
                "object": "morning workouts",
                "confidence": 0.95,
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "remember_fact should succeed: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert!(result["fact_id"].is_string() || result["fact_id"].is_number());
    assert!(result["kind"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_remember_fact_missing_confidence() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "remember_fact",
            json!({
                "kind": "preference",
                "subject": "you",
                "predicate": "prefers",
                "object": "morning workouts",
            }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("missing confidence must be rejected");
    assert_invalid_request(&err, "remember_fact");
    Ok(())
}

#[tokio::test]
async fn test_remember_fact_rejects_no_tenant() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "remember_fact",
            json!({
                "kind": "preference",
                "subject": "you",
                "predicate": "prefers",
                "object": "x",
                "confidence": 0.9,
            }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_tool_errored(&err, "remember_fact");
    Ok(())
}

// ============================================================================
// recall_user_memory
// ============================================================================

#[tokio::test]
async fn test_recall_user_memory_empty() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "recall_user_memory",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success, "recall should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    assert!(result["facts"].as_array().unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn test_recall_user_memory_returns_seeded_fact() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    // Seed a fact via remember_fact, then recall it.
    executor
        .execute_tool(make_request(
            "remember_fact",
            json!({
                "kind": "goal",
                "subject": "you",
                "predicate": "targets",
                "object": "sub-3 marathon",
                "confidence": 0.99,
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;

    let resp = executor
        .execute_tool(make_request(
            "recall_user_memory",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    let count = result["count"].as_u64().unwrap();
    assert!(
        count >= 1,
        "expected at least one recalled fact, got {count}"
    );
    let facts = result["facts"].as_array().unwrap();
    let has_goal = facts.iter().any(|f| {
        f["kind"].as_str() == Some("goal") && f["object"].as_str() == Some("sub-3 marathon")
    });
    assert!(has_goal, "recall must include the seeded goal fact");
    Ok(())
}

#[tokio::test]
async fn test_recall_user_memory_rejects_no_tenant() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request("recall_user_memory", json!({}), user_id, None))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_tool_errored(&err, "recall_user_memory");
    Ok(())
}

// ============================================================================
// verify_claim
// ============================================================================

#[tokio::test]
async fn test_verify_claim_happy_path() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "verify_claim",
            json!({
                "claim": "Running daily improves cardiovascular fitness.",
                "category": "training_prescription",
            }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        resp.success,
        "verify_claim should succeed: {:?}",
        resp.error
    );
    let result = resp.result.unwrap();
    assert!(result["verdict_id"].is_string() || result["verdict_id"].is_number());
    assert!(result["status"].as_str().is_some());
    assert!(result["evidence_strength"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_verify_claim_invalid_category() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "verify_claim",
            json!({
                "claim": "Some claim",
                "category": "not_a_real_category",
            }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("unknown category must be rejected");
    assert_invalid_request(&err, "verify_claim");
    Ok(())
}

#[tokio::test]
async fn test_verify_claim_empty() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "verify_claim",
            json!({
                "claim": "   ",
                "category": "training_prescription",
            }),
            user_id,
            Some(&tenant),
        ))
        .await
        .expect_err("empty claim must be rejected");
    assert_invalid_request(&err, "verify_claim");
    Ok(())
}

#[tokio::test]
async fn test_verify_claim_rejects_no_tenant() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, _tenant) = create_test_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "verify_claim",
            json!({
                "claim": "x",
                "category": "training_prescription",
            }),
            user_id,
            None,
        ))
        .await
        .expect_err("no-tenant call must be rejected");
    assert_tool_errored(&err, "verify_claim");
    Ok(())
}

// ============================================================================
// analyze_weather_impact — provider-gated, returns ToolResult::error on failure
// ============================================================================

#[tokio::test]
async fn test_analyze_weather_impact_missing_activity_id() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    // The tool returns success=false via ToolResult::error when activity_id
    // is missing, rather than propagating an AppError.
    let resp = executor
        .execute_tool(make_request(
            "analyze_weather_impact",
            json!({}),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(
        !resp.success,
        "missing activity_id must surface success=false"
    );
    Ok(())
}

#[tokio::test]
async fn test_analyze_weather_impact_no_provider() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    // With activity_id but no connected provider, the tool returns
    // success=false with a structured error payload rather than panicking.
    let resp = executor
        .execute_tool(make_request(
            "analyze_weather_impact",
            json!({ "activity_id": "12345" }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(!resp.success, "no provider must surface success=false");
    Ok(())
}

#[tokio::test]
async fn test_analyze_weather_impact_units_param() -> Result<()> {
    let executor = create_misc_test_executor().await?;
    let (user_id, tenant) = create_test_user(&executor).await?;

    // Imperial units should also fall through to the provider-missing error
    // path without panicking — confirms the units arg parsing didn't break.
    let resp = executor
        .execute_tool(make_request(
            "analyze_weather_impact",
            json!({ "activity_id": "12345", "units": "imperial" }),
            user_id,
            Some(&tenant),
        ))
        .await?;
    assert!(!resp.success);
    Ok(())
}
