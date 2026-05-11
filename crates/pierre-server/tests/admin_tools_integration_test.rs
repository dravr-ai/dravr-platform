// ABOUTME: Integration tests for admin MCP tool handlers driven through UniversalToolExecutor
// ABOUTME: Covers the 8 admin coach tools end-to-end: happy path, not-found, and non-admin rejection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin Tool Handler Integration Tests
//!
//! Tests the 8 admin MCP tools via the `UniversalToolExecutor`:
//! - `admin_list_system_coaches`
//! - `admin_create_system_coach`
//! - `admin_get_system_coach`
//! - `admin_update_system_coach`
//! - `admin_delete_system_coach`
//! - `admin_assign_coach`
//! - `admin_unassign_coach`
//! - `admin_list_coach_assignments`
//!
//! Each tool is exercised with at least: happy path against an admin caller,
//! a failure path (not-found or missing required arg), and the non-admin
//! rejection path that `verify_admin_access` enforces on every handler.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use chrono::Utc;
use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::permissions::UserRole;
use pierre_mcp_server::protocols::universal::{UniversalRequest, UniversalToolExecutor};
use pierre_mcp_server::protocols::ProtocolError;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

mod common;

// ============================================================================
// Test Setup
// ============================================================================

async fn create_admin_test_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(UniversalToolExecutor::new(resources)))
}

/// Create an admin user with their own tenant. The user is marked `UserRole::Admin`
/// so `verify_admin_access` in the handler module accepts them.
async fn create_admin_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("admin_test_{}@example.com", Uuid::new_v4());
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(email.clone(), password_hash, Some("Test Admin".to_owned()));
    user.is_admin = true;
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    executor.resources.repos.users.create(&user).await?;

    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Admin Tenant for {email}"),
        slug: format!("admin-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    executor.resources.repos.tenants.create(&tenant).await?;
    executor
        .resources
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await?;

    Ok((user_id, tenant_id.to_string()))
}

/// Create a regular (non-admin) user. `verify_admin_access` should reject this caller
/// with `ProtocolError::InvalidRequest("Permission denied: Admin access required")`.
async fn create_regular_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("regular_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(&executor.resources.database, &email).await?;
    let tenants = executor.resources.repos.tenants.get_all().await?;
    let user_tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("regular user should have tenant"))?;
    Ok((user_id, user_tenant.id.to_string()))
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

/// Create a system coach via `admin_create_system_coach` and return its id.
async fn create_system_coach(
    executor: &UniversalToolExecutor,
    admin_id: Uuid,
    admin_tenant: &str,
    title: &str,
) -> Result<String> {
    let resp = executor
        .execute_tool(make_request(
            "admin_create_system_coach",
            json!({
                "title": title,
                "system_prompt": "You are a helpful test coach.",
                "category": "training",
            }),
            admin_id,
            admin_tenant,
        ))
        .await?;
    assert!(resp.success, "create system coach should succeed");
    let id = resp.result.as_ref().unwrap()["id"]
        .as_str()
        .expect("created coach must have id")
        .to_owned();
    Ok(id)
}

/// Assert that a non-admin caller is rejected with the canonical permission error.
fn assert_permission_denied(err: &ProtocolError, tool: &str) {
    match err {
        ProtocolError::InvalidRequest(msg) => {
            assert!(
                msg.contains("Permission denied") || msg.contains("Admin access required"),
                "{tool}: expected permission-denied message, got: {msg}"
            );
        }
        other => panic!("{tool}: expected InvalidRequest, got {other:?}"),
    }
}

// ============================================================================
// Tool registration sanity check
// ============================================================================

#[tokio::test]
async fn test_admin_tools_registered() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let names: Vec<String> = executor
        .resources
        .tool_registry
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    for expected in [
        "admin_list_system_coaches",
        "admin_create_system_coach",
        "admin_get_system_coach",
        "admin_update_system_coach",
        "admin_delete_system_coach",
        "admin_assign_coach",
        "admin_unassign_coach",
        "admin_list_coach_assignments",
    ] {
        assert!(
            names.iter().any(|n| n == expected),
            "tool registry missing {expected}"
        );
    }
    Ok(())
}

// ============================================================================
// admin_list_system_coaches
// ============================================================================

#[tokio::test]
async fn test_admin_list_system_coaches_empty() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_list_system_coaches",
            json!({}),
            admin_id,
            &tenant,
        ))
        .await?;

    assert!(resp.success, "list should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    assert!(result["coaches"].as_array().unwrap().is_empty());
    Ok(())
}

#[tokio::test]
async fn test_admin_list_system_coaches_after_create() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    create_system_coach(&executor, admin_id, &tenant, "Sample Coach A").await?;
    create_system_coach(&executor, admin_id, &tenant, "Sample Coach B").await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_list_system_coaches",
            json!({}),
            admin_id,
            &tenant,
        ))
        .await?;

    assert!(resp.success);
    let result = resp.result.unwrap();
    assert_eq!(result["count"].as_u64().unwrap(), 2);
    let titles: Vec<&str> = result["coaches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Sample Coach A"));
    assert!(titles.contains(&"Sample Coach B"));
    Ok(())
}

#[tokio::test]
async fn test_admin_list_system_coaches_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_list_system_coaches",
            json!({}),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin caller must be rejected");
    assert_permission_denied(&err, "admin_list_system_coaches");
    Ok(())
}

// ============================================================================
// admin_create_system_coach
// ============================================================================

#[tokio::test]
async fn test_admin_create_system_coach_happy_path() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_create_system_coach",
            json!({
                "title": "Marathon Coach",
                "system_prompt": "Periodization expert for marathon runners.",
                "description": "Marathon-focused system coach",
                "category": "training",
                "tags": ["running", "marathon"],
            }),
            admin_id,
            &tenant,
        ))
        .await?;

    assert!(resp.success, "create should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["title"].as_str().unwrap(), "Marathon Coach");
    assert!(result["is_system"].as_bool().unwrap());
    assert!(result["id"].as_str().is_some());
    Ok(())
}

#[tokio::test]
async fn test_admin_create_system_coach_missing_title() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_create_system_coach",
            json!({ "system_prompt": "no title here" }),
            admin_id,
            &tenant,
        ))
        .await
        .expect_err("missing title must reject");
    match err {
        ProtocolError::InvalidRequest(_) => {}
        other => panic!("expected InvalidRequest, got {other:?}"),
    }
    Ok(())
}

#[tokio::test]
async fn test_admin_create_system_coach_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_create_system_coach",
            json!({ "title": "x", "system_prompt": "y" }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_create_system_coach");
    Ok(())
}

// ============================================================================
// admin_get_system_coach
// ============================================================================

#[tokio::test]
async fn test_admin_get_system_coach_happy_path() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "Coach To Fetch").await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_get_system_coach",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await?;

    assert!(resp.success, "get should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    assert_eq!(result["title"].as_str().unwrap(), "Coach To Fetch");
    Ok(())
}

#[tokio::test]
async fn test_admin_get_system_coach_not_found() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    let bogus = Uuid::new_v4().to_string();
    let resp = executor
        .execute_tool(make_request(
            "admin_get_system_coach",
            json!({ "coach_id": bogus }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(!resp.success, "unknown coach_id must return success=false");
    let err_msg = resp
        .error
        .as_deref()
        .expect("not-found response must carry an error message");
    assert!(
        err_msg.contains("not found"),
        "expected 'not found' in error message, got: {err_msg}"
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_get_system_coach_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_get_system_coach",
            json!({ "coach_id": Uuid::new_v4().to_string() }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_get_system_coach");
    Ok(())
}

// ============================================================================
// admin_update_system_coach
// ============================================================================

#[tokio::test]
async fn test_admin_update_system_coach_happy_path() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "Original Title").await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_update_system_coach",
            json!({
                "coach_id": coach_id,
                "title": "Renamed Title",
                "description": "Updated description",
            }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(resp.success, "update should succeed: {:?}", resp.error);

    // Verify via get
    let get = executor
        .execute_tool(make_request(
            "admin_get_system_coach",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert_eq!(
        get.result.unwrap()["title"].as_str().unwrap(),
        "Renamed Title"
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_update_system_coach_not_found() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    let bogus = Uuid::new_v4().to_string();
    let resp = executor
        .execute_tool(make_request(
            "admin_update_system_coach",
            json!({ "coach_id": bogus, "title": "x" }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(!resp.success, "unknown coach_id must return success=false");
    let err_msg = resp.error.as_deref().expect("error message expected");
    assert!(
        err_msg.contains("not found"),
        "expected 'not found' in error, got: {err_msg}"
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_update_system_coach_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_update_system_coach",
            json!({ "coach_id": Uuid::new_v4().to_string(), "title": "x" }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_update_system_coach");
    Ok(())
}

// ============================================================================
// admin_delete_system_coach
// ============================================================================

#[tokio::test]
async fn test_admin_delete_system_coach_happy_path() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "To Delete").await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_delete_system_coach",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(resp.success, "delete should succeed: {:?}", resp.error);

    // Subsequent list should be empty
    let list = executor
        .execute_tool(make_request(
            "admin_list_system_coaches",
            json!({}),
            admin_id,
            &tenant,
        ))
        .await?;
    assert_eq!(list.result.unwrap()["count"].as_u64().unwrap(), 0);
    Ok(())
}

#[tokio::test]
async fn test_admin_delete_system_coach_not_found() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;

    let bogus = Uuid::new_v4().to_string();
    let resp = executor
        .execute_tool(make_request(
            "admin_delete_system_coach",
            json!({ "coach_id": bogus }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(!resp.success, "unknown coach_id must return success=false");
    let err_msg = resp.error.as_deref().expect("error message expected");
    assert!(
        err_msg.contains("not found"),
        "expected 'not found' in error, got: {err_msg}"
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_delete_system_coach_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_delete_system_coach",
            json!({ "coach_id": Uuid::new_v4().to_string() }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_delete_system_coach");
    Ok(())
}

// ============================================================================
// admin_assign_coach
// ============================================================================

#[tokio::test]
async fn test_admin_assign_coach_happy_path() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, admin_tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &admin_tenant, "Assignable").await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_assign_coach",
            json!({
                "coach_id": coach_id,
                "user_id": admin_id.to_string(),
            }),
            admin_id,
            &admin_tenant,
        ))
        .await?;
    assert!(resp.success, "assign should succeed: {:?}", resp.error);
    Ok(())
}

#[tokio::test]
async fn test_admin_assign_coach_missing_user_id() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "x").await?;

    let result = executor
        .execute_tool(make_request(
            "admin_assign_coach",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await;
    assert!(result.is_err(), "missing user_id must fail");
    Ok(())
}

#[tokio::test]
async fn test_admin_assign_coach_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_assign_coach",
            json!({
                "coach_id": Uuid::new_v4().to_string(),
                "user_id": user_id.to_string(),
            }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_assign_coach");
    Ok(())
}

// ============================================================================
// admin_unassign_coach
// ============================================================================

#[tokio::test]
async fn test_admin_unassign_coach_happy_path() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "Reversible").await?;

    // Assign first
    let assign = executor
        .execute_tool(make_request(
            "admin_assign_coach",
            json!({
                "coach_id": coach_id,
                "user_id": admin_id.to_string(),
            }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(assign.success);

    // Then unassign
    let unassign = executor
        .execute_tool(make_request(
            "admin_unassign_coach",
            json!({
                "coach_id": coach_id,
                "user_id": admin_id.to_string(),
            }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(
        unassign.success,
        "unassign should succeed: {:?}",
        unassign.error
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_unassign_coach_missing_user_id() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "x").await?;

    let result = executor
        .execute_tool(make_request(
            "admin_unassign_coach",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await;
    assert!(result.is_err(), "missing user_id must fail");
    Ok(())
}

#[tokio::test]
async fn test_admin_unassign_coach_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_unassign_coach",
            json!({
                "coach_id": Uuid::new_v4().to_string(),
                "user_id": user_id.to_string(),
            }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_unassign_coach");
    Ok(())
}

// ============================================================================
// admin_list_coach_assignments
// ============================================================================

#[tokio::test]
async fn test_admin_list_coach_assignments_empty() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "Unassigned").await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_list_coach_assignments",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(resp.success, "list should succeed: {:?}", resp.error);
    let result = resp.result.unwrap();
    let assignments = result["assignments"]
        .as_array()
        .or_else(|| result["users"].as_array())
        .unwrap_or(&Vec::new())
        .clone();
    assert!(
        assignments.is_empty(),
        "fresh coach should have no assignments, got {assignments:?}"
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_list_coach_assignments_with_assignment() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (admin_id, tenant) = create_admin_user(&executor).await?;
    let coach_id = create_system_coach(&executor, admin_id, &tenant, "Assigned").await?;

    executor
        .execute_tool(make_request(
            "admin_assign_coach",
            json!({
                "coach_id": coach_id,
                "user_id": admin_id.to_string(),
            }),
            admin_id,
            &tenant,
        ))
        .await?;

    let resp = executor
        .execute_tool(make_request(
            "admin_list_coach_assignments",
            json!({ "coach_id": coach_id }),
            admin_id,
            &tenant,
        ))
        .await?;
    assert!(resp.success);
    let result = resp.result.unwrap();
    let assignments = result["assignments"]
        .as_array()
        .or_else(|| result["users"].as_array())
        .expect("list response must contain assignments array");
    assert_eq!(
        assignments.len(),
        1,
        "expected 1 assignment, got {assignments:?}"
    );
    Ok(())
}

#[tokio::test]
async fn test_admin_list_coach_assignments_rejects_non_admin() -> Result<()> {
    let executor = create_admin_test_executor().await?;
    let (user_id, tenant) = create_regular_user(&executor).await?;

    let err = executor
        .execute_tool(make_request(
            "admin_list_coach_assignments",
            json!({ "coach_id": Uuid::new_v4().to_string() }),
            user_id,
            &tenant,
        ))
        .await
        .expect_err("non-admin must be rejected");
    assert_permission_denied(&err, "admin_list_coach_assignments");
    Ok(())
}
