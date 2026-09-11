// ABOUTME: Integration tests for agent MCP tool handlers (custom AI personas)
// ABOUTME: Tests all 10 agent tools via the UniversalToolExecutor for MCP protocol compliance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Agent Tool Handler Integration Tests
//!
//! Tests the 10 agent MCP tools via the `UniversalToolExecutor`:
//! - `list_agents`: List user's agents with filtering
//! - `create_agent`: Create a new custom agent
//! - `get_agent`: Get a specific agent by ID
//! - `update_agent`: Update agent details
//! - `delete_agent`: Delete an agent
//! - `toggle_agent_favorite`: Toggle favorite status
//! - `search_agents`: Search agents by query
//! - `activate_agent`: Set an agent as active
//! - `deactivate_agent`: Deactivate active agent
//! - `get_active_agent`: Get currently active agent

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

mod common;

// ============================================================================
// Test Setup
// ============================================================================

/// Create test executor for agent tool tests
async fn create_agent_test_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();

    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

/// Create a test user with tenant for agents tests
/// Uses common helper that properly creates both user and tenant with foreign key relationship
async fn create_test_user_for_agents(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("coach_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    // Get tenant_id from the tenant where user is owner
    let all_tenants = executor.resources.repos().tenants.get_all().await?;
    let user_tenant = all_tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("User should have a tenant"))?;
    Ok((user_id, user_tenant.id.to_string()))
}

/// Create a test request with user ID and tenant ID
fn create_test_request(
    tool_name: &str,
    parameters: serde_json::Value,
    user_id: Uuid,
    tenant_id: &str,
) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool_name.to_owned(),
        parameters,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

// ============================================================================
// Tool Registration Tests
// ============================================================================

#[tokio::test]
async fn test_agent_tools_registered() -> Result<()> {
    let executor = create_agent_test_executor().await?;

    let tool_names: Vec<String> = executor
        .resources
        .tool_registry()
        .tool_names()
        .iter()
        .map(|n| (*n).to_owned())
        .collect();

    let expected_tools = vec![
        "list_agents",
        "create_agent",
        "get_agent",
        "update_agent",
        "delete_agent",
        "toggle_agent_favorite",
        "search_agents",
        "activate_agent",
        "deactivate_agent",
        "get_active_agent",
    ];

    for expected_tool in expected_tools {
        assert!(
            tool_names.contains(&expected_tool.to_owned()),
            "Missing coach tool: {expected_tool}"
        );
    }

    Ok(())
}

// ============================================================================
// list_agents Tests
// ============================================================================

#[tokio::test]
async fn test_list_agents_empty() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request("list_agents", json!({}), user_id, &tenant_id);

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert!(result["agents"].is_array());
    assert_eq!(result["count"].as_u64().unwrap(), 0);
    assert_eq!(result["total"].as_u64().unwrap(), 0);

    Ok(())
}

#[tokio::test]
async fn test_list_agents_after_create() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent first
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Test Marathon Coach",
            "system_prompt": "You are a marathon training specialist.",
            "category": "training"
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    assert!(create_response.success);

    // Now list agents
    let list_request = create_test_request("list_agents", json!({}), user_id, &tenant_id);

    let response = executor.execute_tool(list_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["count"].as_u64().unwrap(), 1);
    assert_eq!(result["total"].as_u64().unwrap(), 1);
    let agents = result["agents"].as_array().unwrap();
    assert_eq!(agents[0]["title"].as_str().unwrap(), "Test Marathon Coach");

    Ok(())
}

#[tokio::test]
async fn test_list_agents_with_category_filter() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create agents with different categories
    for (title, category) in [
        ("Training Coach", "training"),
        ("Nutrition Coach", "nutrition"),
        ("Recovery Coach", "recovery"),
    ] {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": title,
                "system_prompt": format!("You are a {} specialist.", category),
                "category": category
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // Filter by training category
    let request = create_test_request(
        "list_agents",
        json!({
            "category": "training"
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    // count = filtered count, total = all agents
    assert_eq!(result["count"].as_u64().unwrap(), 1);
    assert_eq!(result["total"].as_u64().unwrap(), 3);

    Ok(())
}

#[tokio::test]
async fn test_list_agents_with_pagination() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create 5 agents
    for i in 0..5 {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": format!("Coach {}", i),
                "system_prompt": "Generic coach prompt."
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // Get first 2 agents
    let request = create_test_request(
        "list_agents",
        json!({
            "limit": 2,
            "offset": 0
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["count"].as_u64().unwrap(), 2);
    assert_eq!(result["total"].as_u64().unwrap(), 5);
    assert!(result["has_more"].as_bool().unwrap());

    Ok(())
}

#[tokio::test]
async fn test_list_agents_favorites_only() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create two agents
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Favorite Coach",
            "system_prompt": "Prompt"
        }),
        user_id,
        &tenant_id,
    );
    let create_response = executor.execute_tool(create_request).await?;
    let favorite_coach_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let create_request2 = create_test_request(
        "create_agent",
        json!({
            "title": "Regular Coach",
            "system_prompt": "Prompt"
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(create_request2).await?;

    // Toggle favorite on first agent
    let toggle_request = create_test_request(
        "toggle_agent_favorite",
        json!({
            "agent_id": favorite_coach_id
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(toggle_request).await?;

    // List favorites only
    let request = create_test_request(
        "list_agents",
        json!({
            "favorites_only": true
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["count"].as_u64().unwrap(), 1);
    let agents = result["agents"].as_array().unwrap();
    assert_eq!(agents[0]["title"].as_str().unwrap(), "Favorite Coach");

    Ok(())
}

// ============================================================================
// create_agent Tests
// ============================================================================

#[tokio::test]
async fn test_create_agent_success() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "create_agent",
        json!({
            "title": "Elite Running Coach",
            "description": "Specializes in marathon and ultra training",
            "system_prompt": "You are an elite running coach with 20 years of experience training professional marathon runners. Focus on periodization, recovery, and race strategy.",
            "category": "training",
            "tags": ["running", "marathon", "elite", "periodization"]
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(
        response.success,
        "Create should succeed: {:?}",
        response.error
    );
    let result = response.result.unwrap();

    // Verify all returned fields
    assert!(result["id"].is_string(), "Should return coach id");
    assert_eq!(result["title"].as_str().unwrap(), "Elite Running Coach");
    assert_eq!(result["category"].as_str().unwrap(), "training");
    assert!(result["token_count"].as_u64().unwrap() > 0);
    assert!(result["created_at"].is_string());

    // Verify tags
    let tags = result["tags"].as_array().unwrap();
    assert!(tags.iter().any(|t| t.as_str().unwrap() == "marathon"));

    Ok(())
}

#[tokio::test]
async fn test_create_agent_minimal() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Minimal required fields only
    let request = create_test_request(
        "create_agent",
        json!({
            "title": "Simple Coach",
            "system_prompt": "A basic coaching prompt."
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(
        response.success,
        "Create should succeed with minimal fields"
    );
    let result = response.result.unwrap();

    assert!(result["id"].is_string());
    assert_eq!(result["category"].as_str().unwrap(), "custom");

    Ok(())
}

#[tokio::test]
async fn test_create_agent_missing_title() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "create_agent",
        json!({
            "system_prompt": "A prompt without a title."
        }),
        user_id,
        &tenant_id,
    );

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail without title");

    Ok(())
}

#[tokio::test]
async fn test_create_agent_missing_system_prompt() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "create_agent",
        json!({
            "title": "Coach Without Prompt"
        }),
        user_id,
        &tenant_id,
    );

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail without system_prompt");

    Ok(())
}

// ============================================================================
// get_agent Tests
// ============================================================================

#[tokio::test]
async fn test_get_agent_success() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent first
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Detailed Coach",
            "description": "A coach with full details",
            "system_prompt": "You are a detailed coaching assistant.",
            "category": "nutrition",
            "tags": ["detailed", "nutrition"]
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    assert!(create_response.success);
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Get the agent
    let get_request = create_test_request(
        "get_agent",
        json!({
            "agent_id": agent_id
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(get_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    // Verify all fields including system_prompt (only in get_agent response)
    assert_eq!(result["id"].as_str().unwrap(), agent_id);
    assert_eq!(result["title"].as_str().unwrap(), "Detailed Coach");
    assert_eq!(
        result["description"].as_str().unwrap(),
        "A coach with full details"
    );
    assert!(result["system_prompt"]
        .as_str()
        .unwrap()
        .contains("detailed coaching"));
    assert_eq!(result["category"].as_str().unwrap(), "nutrition");
    assert!(result["created_at"].is_string());
    assert!(result["updated_at"].is_string());

    Ok(())
}

#[tokio::test]
async fn test_get_agent_not_found() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "get_agent",
        json!({
            "agent_id": Uuid::new_v4().to_string()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail for nonexistent coach");
    assert!(response.error.unwrap().contains("not found"));

    Ok(())
}

#[tokio::test]
async fn test_get_agent_missing_id() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request("get_agent", json!({}), user_id, &tenant_id);

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail without coach_id");

    Ok(())
}

// ============================================================================
// update_agent Tests
// ============================================================================

#[tokio::test]
async fn test_update_agent_success() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent first
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Original Title",
            "system_prompt": "Original prompt."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Update the agent
    let update_request = create_test_request(
        "update_agent",
        json!({
            "agent_id": agent_id,
            "title": "Updated Title",
            "description": "Added description",
            "system_prompt": "Updated coaching prompt with more details.",
            "category": "training",
            "tags": ["updated", "training"]
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(update_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["title"].as_str().unwrap(), "Updated Title");
    assert_eq!(result["description"].as_str().unwrap(), "Added description");
    assert!(result["system_prompt"]
        .as_str()
        .unwrap()
        .contains("Updated"));
    assert_eq!(result["category"].as_str().unwrap(), "training");

    Ok(())
}

#[tokio::test]
async fn test_update_agent_partial() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent first
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Partial Update Test",
            "system_prompt": "Original prompt.",
            "category": "nutrition"
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Partial update - only title
    let update_request = create_test_request(
        "update_agent",
        json!({
            "agent_id": agent_id,
            "title": "Only Title Updated"
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(update_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["title"].as_str().unwrap(), "Only Title Updated");
    // Category should remain unchanged
    assert_eq!(result["category"].as_str().unwrap(), "nutrition");

    Ok(())
}

#[tokio::test]
async fn test_update_agent_not_found() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "update_agent",
        json!({
            "agent_id": Uuid::new_v4().to_string(),
            "title": "New Title"
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail for nonexistent coach");
    assert!(response.error.unwrap().contains("not found"));

    Ok(())
}

// ============================================================================
// delete_agent Tests
// ============================================================================

#[tokio::test]
async fn test_delete_agent_success() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent first
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Coach to Delete",
            "system_prompt": "This coach will be deleted."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Delete the agent
    let delete_request = create_test_request(
        "delete_agent",
        json!({
            "agent_id": agent_id.clone()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(delete_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert!(result["deleted"].as_bool().unwrap());
    assert_eq!(result["agent_id"].as_str().unwrap(), agent_id);

    // Verify it's gone
    let get_request = create_test_request(
        "get_agent",
        json!({
            "agent_id": agent_id
        }),
        user_id,
        &tenant_id,
    );

    let get_response = executor.execute_tool(get_request).await?;
    assert!(!get_response.success, "Coach should be deleted");

    Ok(())
}

#[tokio::test]
async fn test_delete_agent_not_found() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "delete_agent",
        json!({
            "agent_id": Uuid::new_v4().to_string()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail for nonexistent coach");
    assert!(response.error.unwrap().contains("not found"));

    Ok(())
}

#[tokio::test]
async fn test_delete_agent_missing_id() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request("delete_agent", json!({}), user_id, &tenant_id);

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail without coach_id");

    Ok(())
}

// ============================================================================
// toggle_agent_favorite Tests
// ============================================================================

#[tokio::test]
async fn test_toggle_agent_favorite_on() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Favorite Toggle Test",
            "system_prompt": "Test prompt."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Toggle favorite on
    let toggle_request = create_test_request(
        "toggle_agent_favorite",
        json!({
            "agent_id": agent_id.clone()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(toggle_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert!(result["is_favorite"].as_bool().unwrap());
    assert_eq!(result["agent_id"].as_str().unwrap(), agent_id);

    Ok(())
}

#[tokio::test]
async fn test_toggle_agent_favorite_off() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent and toggle favorite on
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Double Toggle Test",
            "system_prompt": "Test prompt."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // First toggle - on
    let toggle_request1 = create_test_request(
        "toggle_agent_favorite",
        json!({
            "agent_id": agent_id.clone()
        }),
        user_id,
        &tenant_id,
    );
    let response1 = executor.execute_tool(toggle_request1).await?;
    assert!(response1.result.unwrap()["is_favorite"].as_bool().unwrap());

    // Second toggle - off
    let toggle_request2 = create_test_request(
        "toggle_agent_favorite",
        json!({
            "agent_id": agent_id.clone()
        }),
        user_id,
        &tenant_id,
    );
    let response2 = executor.execute_tool(toggle_request2).await?;
    assert!(!response2.result.unwrap()["is_favorite"].as_bool().unwrap());

    Ok(())
}

#[tokio::test]
async fn test_toggle_agent_favorite_not_found() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "toggle_agent_favorite",
        json!({
            "agent_id": Uuid::new_v4().to_string()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail for nonexistent coach");
    assert!(response.error.unwrap().contains("not found"));

    Ok(())
}

// ============================================================================
// search_agents Tests
// ============================================================================

#[tokio::test]
async fn test_search_agents_by_title() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create agents with different titles
    for title in ["Marathon Runner", "Sprint Coach", "Recovery Expert"] {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": title,
                "system_prompt": format!("Specialist in {}.", title.to_lowercase())
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // Search for "marathon"
    let request = create_test_request(
        "search_agents",
        json!({
            "query": "marathon"
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["query"].as_str().unwrap(), "marathon");
    assert_eq!(result["returned_count"].as_u64().unwrap(), 1);

    let results = result["results"].as_array().unwrap();
    assert!(results[0]["title"].as_str().unwrap().contains("Marathon"));

    Ok(())
}

#[tokio::test]
async fn test_search_agents_by_tag() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create agents with tags
    let request1 = create_test_request(
        "create_agent",
        json!({
            "title": "HIIT Coach",
            "system_prompt": "High intensity training.",
            "tags": ["hiit", "cardio", "intense"]
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(request1).await?;

    let request2 = create_test_request(
        "create_agent",
        json!({
            "title": "Yoga Coach",
            "system_prompt": "Flexibility and mindfulness.",
            "tags": ["yoga", "flexibility", "calm"]
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(request2).await?;

    // Search for "hiit" tag
    let request = create_test_request(
        "search_agents",
        json!({
            "query": "hiit"
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["returned_count"].as_u64().unwrap(), 1);

    Ok(())
}

#[tokio::test]
async fn test_search_agents_no_results() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "search_agents",
        json!({
            "query": "nonexistent_coach_xyz"
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["returned_count"].as_u64().unwrap(), 0);
    assert!(result["results"].as_array().unwrap().is_empty());

    Ok(())
}

#[tokio::test]
async fn test_search_agents_missing_query() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request("search_agents", json!({}), user_id, &tenant_id);

    let result = executor.execute_tool(request).await;

    assert!(result.is_err(), "Should fail without query");

    Ok(())
}

#[tokio::test]
async fn test_search_agents_with_limit() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create 10 agents with "test" in the title
    for i in 0..10 {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": format!("Test Coach {}", i),
                "system_prompt": "A test coach."
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // Search with limit
    let request = create_test_request(
        "search_agents",
        json!({
            "query": "test",
            "limit": 3
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["returned_count"].as_u64().unwrap(), 3);

    Ok(())
}

#[tokio::test]
async fn test_search_agents_pagination_has_more_true() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create 5 agents
    for i in 0..5 {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": format!("Pagination Coach {}", i),
                "system_prompt": "Testing pagination."
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // Search with limit=3, expecting has_more=true (5 agents, returning 3)
    let request = create_test_request(
        "search_agents",
        json!({
            "query": "pagination",
            "limit": 3
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;
    assert!(response.success);
    let result = response.result.unwrap();

    // Verify pagination metadata fields exist and are correct
    assert_eq!(result["returned_count"].as_u64().unwrap(), 3);
    assert_eq!(result["limit"].as_u64().unwrap(), 3);
    assert_eq!(result["offset"].as_u64().unwrap(), 0);
    assert!(
        result["has_more"].as_bool().unwrap(),
        "has_more should be true when returned_count equals limit"
    );

    Ok(())
}

#[tokio::test]
async fn test_search_agents_pagination_has_more_false() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create 2 agents
    for i in 0..2 {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": format!("Limited Coach {}", i),
                "system_prompt": "Testing has_more false."
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // Search with limit=10, expecting has_more=false (only 2 agents)
    let request = create_test_request(
        "search_agents",
        json!({
            "query": "limited",
            "limit": 10
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;
    assert!(response.success);
    let result = response.result.unwrap();

    assert_eq!(result["returned_count"].as_u64().unwrap(), 2);
    assert_eq!(result["limit"].as_u64().unwrap(), 10);
    assert!(
        !result["has_more"].as_bool().unwrap(),
        "has_more should be false when returned_count < limit"
    );

    Ok(())
}

#[tokio::test]
async fn test_search_agents_pagination_with_offset() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create 5 agents with sequential titles for deterministic ordering
    for i in 0..5 {
        let request = create_test_request(
            "create_agent",
            json!({
                "title": format!("Offset Coach {}", i),
                "system_prompt": "Testing offset pagination."
            }),
            user_id,
            &tenant_id,
        );
        executor.execute_tool(request).await?;
    }

    // First page: offset=0, limit=2
    let request1 = create_test_request(
        "search_agents",
        json!({
            "query": "offset",
            "limit": 2,
            "offset": 0
        }),
        user_id,
        &tenant_id,
    );

    let response1 = executor.execute_tool(request1).await?;
    assert!(response1.success);
    let result1 = response1.result.unwrap();

    assert_eq!(result1["returned_count"].as_u64().unwrap(), 2);
    assert_eq!(result1["offset"].as_u64().unwrap(), 0);
    assert!(result1["has_more"].as_bool().unwrap());

    // Second page: offset=2, limit=2
    let request2 = create_test_request(
        "search_agents",
        json!({
            "query": "offset",
            "limit": 2,
            "offset": 2
        }),
        user_id,
        &tenant_id,
    );

    let response2 = executor.execute_tool(request2).await?;
    assert!(response2.success);
    let result2 = response2.result.unwrap();

    assert_eq!(result2["returned_count"].as_u64().unwrap(), 2);
    assert_eq!(result2["offset"].as_u64().unwrap(), 2);
    assert!(result2["has_more"].as_bool().unwrap());

    // Third page: offset=4, limit=2 (only 1 remaining)
    let request3 = create_test_request(
        "search_agents",
        json!({
            "query": "offset",
            "limit": 2,
            "offset": 4
        }),
        user_id,
        &tenant_id,
    );

    let response3 = executor.execute_tool(request3).await?;
    assert!(response3.success);
    let result3 = response3.result.unwrap();

    assert_eq!(result3["returned_count"].as_u64().unwrap(), 1);
    assert_eq!(result3["offset"].as_u64().unwrap(), 4);
    assert!(
        !result3["has_more"].as_bool().unwrap(),
        "has_more should be false on last page"
    );

    Ok(())
}

// ============================================================================
// activate_agent / deactivate_agent / get_active_agent Tests
// ============================================================================

#[tokio::test]
async fn test_activate_agent_success() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Coach to Activate",
            "system_prompt": "This coach will be activated."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Activate the agent
    let activate_request = create_test_request(
        "activate_agent",
        json!({
            "agent_id": agent_id.clone()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(activate_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert_eq!(result["id"].as_str().unwrap(), agent_id);
    assert!(result["is_active"].as_bool().unwrap());
    assert!(result["system_prompt"].is_string()); // Activation returns full details

    Ok(())
}

#[tokio::test]
async fn test_activate_agent_not_found() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    let request = create_test_request(
        "activate_agent",
        json!({
            "agent_id": Uuid::new_v4().to_string()
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(!response.success, "Should fail for nonexistent coach");
    assert!(response.error.unwrap().contains("not found"));

    Ok(())
}

#[tokio::test]
async fn test_deactivate_agent_success() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create and activate an agent
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Coach to Deactivate",
            "system_prompt": "This coach will be deactivated."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Activate first
    let activate_request = create_test_request(
        "activate_agent",
        json!({
            "agent_id": agent_id
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(activate_request).await?;

    // Now deactivate
    let deactivate_request =
        create_test_request("deactivate_agent", json!({}), user_id, &tenant_id);

    let response = executor.execute_tool(deactivate_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert!(result["deactivated"].as_bool().unwrap());

    Ok(())
}

#[tokio::test]
async fn test_deactivate_agent_when_none_active() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Deactivate when nothing is active
    let deactivate_request =
        create_test_request("deactivate_agent", json!({}), user_id, &tenant_id);

    let response = executor.execute_tool(deactivate_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert!(!result["deactivated"].as_bool().unwrap());

    Ok(())
}

#[tokio::test]
async fn test_get_active_agent_when_active() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create and activate an agent
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "Active Coach Test",
            "description": "Testing get_active_agent",
            "system_prompt": "This is the active coach prompt."
        }),
        user_id,
        &tenant_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let activate_request = create_test_request(
        "activate_agent",
        json!({
            "agent_id": agent_id.clone()
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(activate_request).await?;

    // Get active agent
    let get_active_request =
        create_test_request("get_active_agent", json!({}), user_id, &tenant_id);

    let response = executor.execute_tool(get_active_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert!(result["active"].as_bool().unwrap());

    let agent = &result["agent"];
    assert_eq!(agent["id"].as_str().unwrap(), agent_id);
    assert_eq!(agent["title"].as_str().unwrap(), "Active Coach Test");
    assert!(agent["system_prompt"]
        .as_str()
        .unwrap()
        .contains("active coach prompt"));

    Ok(())
}

#[tokio::test]
async fn test_get_active_agent_when_none_active() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Get active agent when none is active
    let get_active_request =
        create_test_request("get_active_agent", json!({}), user_id, &tenant_id);

    let response = executor.execute_tool(get_active_request).await?;

    assert!(response.success);
    let result = response.result.unwrap();
    assert!(!result["active"].as_bool().unwrap());
    // `Value` indexes a missing key to Null, so is_null() alone would also
    // pass if the field were renamed out from under this test. Name the key.
    assert!(
        result.get("agent").is_some(),
        "get_active_agent always declares the agent field, null or not"
    );
    assert!(result["agent"].is_null());

    Ok(())
}

#[tokio::test]
async fn test_activate_replaces_previous_active() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create two agents
    let create_request1 = create_test_request(
        "create_agent",
        json!({
            "title": "First Coach",
            "system_prompt": "First coach prompt."
        }),
        user_id,
        &tenant_id,
    );
    let create_response1 = executor.execute_tool(create_request1).await?;
    let coach1_id = create_response1.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let create_request2 = create_test_request(
        "create_agent",
        json!({
            "title": "Second Coach",
            "system_prompt": "Second coach prompt."
        }),
        user_id,
        &tenant_id,
    );
    let create_response2 = executor.execute_tool(create_request2).await?;
    let coach2_id = create_response2.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // Activate first agent
    let activate_request1 = create_test_request(
        "activate_agent",
        json!({
            "agent_id": coach1_id
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(activate_request1).await?;

    // Verify first is active
    let get_active_request1 =
        create_test_request("get_active_agent", json!({}), user_id, &tenant_id);
    let response1 = executor.execute_tool(get_active_request1).await?;
    assert_eq!(
        response1.result.unwrap()["agent"]["title"]
            .as_str()
            .unwrap(),
        "First Coach"
    );

    // Activate second agent (should replace first)
    let activate_request2 = create_test_request(
        "activate_agent",
        json!({
            "agent_id": coach2_id
        }),
        user_id,
        &tenant_id,
    );
    executor.execute_tool(activate_request2).await?;

    // Verify second is now active
    let get_active_request2 =
        create_test_request("get_active_agent", json!({}), user_id, &tenant_id);
    let response2 = executor.execute_tool(get_active_request2).await?;
    assert_eq!(
        response2.result.unwrap()["agent"]["title"]
            .as_str()
            .unwrap(),
        "Second Coach"
    );

    Ok(())
}

// ============================================================================
// User Isolation Tests
// ============================================================================

#[tokio::test]
async fn test_agent_user_isolation() -> Result<()> {
    let executor = create_agent_test_executor().await?;

    // Create two users with their own tenants
    let (user1_id, tenant1_id) = create_test_user_for_agents(&executor).await?;
    let (user2_id, tenant2_id) = create_test_user_for_agents(&executor).await?;

    // User 1 creates an agent
    let create_request = create_test_request(
        "create_agent",
        json!({
            "title": "User 1 Secret Coach",
            "system_prompt": "User 1's private coaching prompt."
        }),
        user1_id,
        &tenant1_id,
    );

    let create_response = executor.execute_tool(create_request).await?;
    assert!(create_response.success);
    let agent_id = create_response.result.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // User 2 should not see User 1's agent in list
    let list_request = create_test_request("list_agents", json!({}), user2_id, &tenant2_id);

    let response = executor.execute_tool(list_request).await?;
    let result = response.result.unwrap();

    assert_eq!(
        result["count"].as_u64().unwrap(),
        0,
        "User 2 should not see User 1's coaches"
    );

    // User 2 should not be able to get User 1's agent
    let get_request = create_test_request(
        "get_agent",
        json!({
            "agent_id": agent_id.clone()
        }),
        user2_id,
        &tenant2_id,
    );

    let get_response = executor.execute_tool(get_request).await?;
    assert!(
        !get_response.success,
        "User 2 should not access User 1's coach"
    );

    // User 2 should not be able to delete User 1's agent
    let delete_request = create_test_request(
        "delete_agent",
        json!({
            "agent_id": agent_id.clone()
        }),
        user2_id,
        &tenant2_id,
    );

    let delete_response = executor.execute_tool(delete_request).await?;
    assert!(
        !delete_response.success,
        "User 2 should not delete User 1's coach"
    );

    // User 2 should not be able to activate User 1's agent
    let activate_request = create_test_request(
        "activate_agent",
        json!({
            "agent_id": agent_id
        }),
        user2_id,
        &tenant2_id,
    );

    let activate_response = executor.execute_tool(activate_request).await?;
    assert!(
        !activate_response.success,
        "User 2 should not activate User 1's coach"
    );

    Ok(())
}

// ============================================================================
// Token Count Tests
// ============================================================================

#[tokio::test]
async fn test_agent_token_count_calculated() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Create an agent with a long system prompt
    let long_prompt = "You are an experienced marathon coach with over 25 years of experience \
        training athletes of all levels. Your approach combines scientific training principles \
        with practical race-day wisdom. You focus on periodization, recovery, nutrition timing, \
        and mental preparation. You always consider the athlete's current fitness level, \
        goals, and available training time when creating personalized plans.";

    let request = create_test_request(
        "create_agent",
        json!({
            "title": "Token Count Test Coach",
            "description": "Testing token count calculation",
            "system_prompt": long_prompt
        }),
        user_id,
        &tenant_id,
    );

    let response = executor.execute_tool(request).await?;

    assert!(response.success);
    let result = response.result.unwrap();

    let token_count = result["token_count"].as_u64().unwrap();
    assert!(
        token_count > 50,
        "Token count should be > 50 for long prompt: {token_count}"
    );
    assert!(
        token_count < 500,
        "Token count should be reasonable: {token_count}"
    );

    Ok(())
}

// ============================================================================
// Hidden-set Tenant Gate Tests
// ============================================================================

/// `show_agent` refuses a caller with no resolved tenant exactly as
/// `hide_agent` does. The hidden-set row is keyed per user, so the tenant is a
/// gate on the handler rather than a key on the row — and a refused call must
/// leave the row alone, where a tenant-bearing caller removes it.
#[tokio::test]
async fn test_show_agent_without_tenant_is_refused_and_keeps_the_agent_hidden() -> Result<()> {
    let executor = create_agent_test_executor().await?;
    let (user_id, tenant_id) = create_test_user_for_agents(&executor).await?;

    // Only a system or assigned agent is hideable; a personal one is refused.
    let system_coach = executor
        .resources
        .repos()
        .agents
        .create_system_agent(
            user_id,
            TenantId::from_uuid(Uuid::parse_str(&tenant_id)?),
            &CreateSystemAgentRequest {
                title: "Gated Coach".to_owned(),
                description: None,
                system_prompt: "You are a coach.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await?;
    let agent_id = system_coach.id.to_string();
    let args = json!({ "agent_id": agent_id });

    let mut tenantless_hide = create_test_request("hide_agent", args.clone(), user_id, &tenant_id);
    tenantless_hide.tenant_id = None;
    let refused = executor
        .execute_tool(tenantless_hide)
        .await
        .expect_err("hide_agent must refuse a caller with no tenant");
    assert!(
        refused.to_string().contains("Tenant context required"),
        "unexpected hide refusal: {refused}"
    );

    let hidden = executor
        .execute_tool(create_test_request(
            "hide_agent",
            args.clone(),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert!(hidden.success, "hide_agent failed: {:?}", hidden.error);

    let mut tenantless_show = create_test_request("show_agent", args.clone(), user_id, &tenant_id);
    tenantless_show.tenant_id = None;
    let refused = executor
        .execute_tool(tenantless_show)
        .await
        .expect_err("show_agent must refuse a caller with no tenant");
    assert!(
        refused.to_string().contains("Tenant context required"),
        "unexpected show refusal: {refused}"
    );

    // The refusal wrote nothing: the agent is still in the hidden-set.
    let still_hidden = executor
        .execute_tool(create_test_request(
            "list_hidden_agents",
            json!({}),
            user_id,
            &tenant_id,
        ))
        .await?;
    let still_hidden = still_hidden.result.unwrap();
    let hidden_ids: Vec<&str> = still_hidden["agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(hidden_ids, vec![agent_id.as_str()]);

    // The tenant-bearing caller removes the preference and the agent is back.
    let shown = executor
        .execute_tool(create_test_request("show_agent", args, user_id, &tenant_id))
        .await?;
    assert!(shown.success, "show_agent failed: {:?}", shown.error);
    let shown = shown.result.unwrap();
    assert_eq!(shown["removed_preference"], json!(true));
    assert_eq!(shown["is_hidden"], json!(false));

    let none_hidden = executor
        .execute_tool(create_test_request(
            "list_hidden_agents",
            json!({}),
            user_id,
            &tenant_id,
        ))
        .await?;
    assert_eq!(none_hidden.result.unwrap()["count"].as_u64(), Some(0));

    Ok(())
}
