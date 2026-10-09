// ABOUTME: Integration tests for the agents route handlers
// ABOUTME: Tests agent read/update/delete, favorites filtering, usage tracking, and authentication flows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{
    create_test_server_resources, create_test_user, create_test_user_with_email,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::TenantId;
use pierre_database::database::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::agents::{AgentResponse, ListAgentsResponse, RecordUsageResponse};
use pierre_routes_agents::build_agents_router;

use axum::http::StatusCode;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

// ============================================================================
// Test Helpers
// ============================================================================

async fn setup_test_environment() -> (axum::Router, String, Seed) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .map(|t| t.id)
        .expect("test user owns a tenant");

    // Generate a JWT token for the user
    let token = generate_test_token(&resources, &user).await;

    let seed = Seed {
        resources: Arc::clone(&resources),
        user_id,
        tenant_id,
    };

    // Create the agents router
    let router = build_agents_router::<ServerContext>().with_state(resources);

    (router, format!("Bearer {token}"), seed)
}

/// Writes the fixtures a test reads back through the routes, through the
/// repository every agent-creation surface stores with.
struct Seed {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
}

impl Seed {
    async fn agent(&self, body: serde_json::Value) -> AgentResponse {
        let request: CreateAgentRequest = serde_json::from_value(body).unwrap();
        self.resources
            .common
            .repos
            .agents
            .create(self.user_id, self.tenant_id, &request)
            .await
            .unwrap()
            .into()
    }

    async fn favorite(&self, agent_id: &str) {
        self.resources
            .common
            .repos
            .agents
            .toggle_favorite(agent_id, self.user_id, self.tenant_id)
            .await
            .unwrap()
            .expect("the agent exists");
    }
}

// ============================================================================
// Agent CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_list_agents() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create an agent first
    seed.agent(json!({
        "title": "Test Coach",
        "system_prompt": "Test prompt"
    }))
    .await;

    // List agents
    let list_response = AxumTestRequest::get("/api/agents")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(list_response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = list_response.json();
    assert_eq!(list.total, 1);
    assert_eq!(list.agents.len(), 1);
    assert_eq!(list.agents[0].title, "Test Coach");
}

#[tokio::test]
async fn test_get_agent() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create an agent first
    let created = seed
        .agent(json!({
            "title": "Get Test Coach",
            "system_prompt": "Test prompt"
        }))
        .await;

    // Get the agent
    let get_response = AxumTestRequest::get(&format!("/api/agents/{}", created.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(get_response.status_code(), StatusCode::OK);

    let agent: AgentResponse = get_response.json();
    assert_eq!(agent.id, created.id);
    assert_eq!(agent.title, "Get Test Coach");
}

#[tokio::test]
async fn test_update_agent() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create an agent first
    let created = seed
        .agent(json!({
            "title": "Original Title",
            "system_prompt": "Original prompt"
        }))
        .await;

    // Update the agent
    let update_response = AxumTestRequest::put(&format!("/api/agents/{}", created.id))
        .header("authorization", &auth_token)
        .json(&json!({
            "title": "Updated Title",
            "system_prompt": "Updated prompt",
            "category": "nutrition"
        }))
        .send(router.clone())
        .await;

    assert_eq!(update_response.status_code(), StatusCode::OK);

    // Verify the update
    let get_response = AxumTestRequest::get(&format!("/api/agents/{}", created.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    let agent: AgentResponse = get_response.json();
    assert_eq!(agent.title, "Updated Title");
    assert_eq!(agent.system_prompt, "Updated prompt");
    assert_eq!(agent.category, "nutrition");
}

#[tokio::test]
async fn test_delete_agent() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create an agent first
    let created = seed
        .agent(json!({
            "title": "To Delete",
            "system_prompt": "Will be deleted"
        }))
        .await;

    // Delete the agent
    let delete_response = AxumTestRequest::delete(&format!("/api/agents/{}", created.id))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(delete_response.status_code(), StatusCode::NO_CONTENT);

    // Verify deletion - should return 404
    let get_response = AxumTestRequest::get(&format!("/api/agents/{}", created.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(get_response.status_code(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Favorites Tests
// ============================================================================

#[tokio::test]
async fn test_list_favorites_only() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create two agents
    let coach1 = seed
        .agent(json!({
            "title": "Coach 1",
            "system_prompt": "Prompt 1"
        }))
        .await;

    seed.agent(json!({
        "title": "Coach 2",
        "system_prompt": "Prompt 2"
    }))
    .await;

    // Mark coach1 as favorite
    seed.favorite(&coach1.id).await;

    // List only favorites
    let list_response = AxumTestRequest::get("/api/agents?favorites_only=true")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(list_response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = list_response.json();
    // total shows all agents, agents.len() shows filtered result
    assert_eq!(list.total, 2);
    assert_eq!(list.agents.len(), 1);
    assert_eq!(list.agents[0].title, "Coach 1");
}

// ============================================================================
// Usage Tracking Tests
// ============================================================================

#[tokio::test]
async fn test_record_usage() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create an agent
    let created = seed
        .agent(json!({
            "title": "Usage Test",
            "system_prompt": "Test prompt"
        }))
        .await;
    assert_eq!(created.use_count, 0);

    // Record usage
    let usage_response = AxumTestRequest::post(&format!("/api/agents/{}/usage", created.id))
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    assert_eq!(usage_response.status_code(), StatusCode::OK);
    let usage_result: RecordUsageResponse = usage_response.json();
    assert!(usage_result.success);

    // Verify use_count increased
    let get_response = AxumTestRequest::get(&format!("/api/agents/{}", created.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    let agent: AgentResponse = get_response.json();
    assert_eq!(agent.use_count, 1);
}

// ============================================================================
// Category Filter Tests
// ============================================================================

#[tokio::test]
async fn test_list_by_category() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create agents in different categories
    seed.agent(json!({
        "title": "Training Coach",
        "system_prompt": "Training",
        "category": "training"
    }))
    .await;

    seed.agent(json!({
        "title": "Nutrition Coach",
        "system_prompt": "Nutrition",
        "category": "nutrition"
    }))
    .await;

    // List only training agents
    let list_response = AxumTestRequest::get("/api/agents?category=training")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(list_response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = list_response.json();
    // total shows all agents, agents.len() shows filtered result
    assert_eq!(list.total, 2);
    assert_eq!(list.agents.len(), 1);
    assert_eq!(list.agents[0].category, "training");
}

// ============================================================================
// Pagination Tests
// ============================================================================

#[tokio::test]
async fn test_list_agents_pagination() {
    let (router, auth_token, seed) = setup_test_environment().await;

    // Create 3 agents (max_coaches_per_user default quota is 3)
    for i in 1..=3 {
        seed.agent(json!({
            "title": format!("Coach {}", i),
            "system_prompt": format!("Prompt {}", i)
        }))
        .await;
    }

    // Get first page (limit=2)
    let page1_response = AxumTestRequest::get("/api/agents?limit=2&offset=0")
        .header("authorization", &auth_token)
        .send(router.clone())
        .await;

    let page1: ListAgentsResponse = page1_response.json();
    assert_eq!(page1.agents.len(), 2);
    assert_eq!(page1.total, 3);

    // Get second page
    let page2_response = AxumTestRequest::get("/api/agents?limit=2&offset=2")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    let page2: ListAgentsResponse = page2_response.json();
    assert_eq!(page2.agents.len(), 1);
}

// ============================================================================
// Authentication Tests
// ============================================================================

// ============================================================================
// Not Found Tests
// ============================================================================

#[tokio::test]
async fn test_get_nonexistent_agent() {
    let (router, auth_token, _) = setup_test_environment().await;

    let response = AxumTestRequest::get("/api/agents/nonexistent-id")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_nonexistent_agent() {
    let (router, auth_token, _) = setup_test_environment().await;

    let response = AxumTestRequest::put("/api/agents/nonexistent-id")
        .header("authorization", &auth_token)
        .json(&json!({
            "title": "New Title"
        }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_nonexistent_agent() {
    let (router, auth_token, _) = setup_test_environment().await;

    let response = AxumTestRequest::delete("/api/agents/nonexistent-id")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_record_usage_nonexistent() {
    let (router, auth_token, _) = setup_test_environment().await;

    let response = AxumTestRequest::post("/api/agents/nonexistent-id/usage")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}

// ============================================================================
// System Agent Cross-Tenant Visibility E2E Tests
// ============================================================================

/// E2E test: System agents should be visible to users via the API
/// This tests the full flow from database to API response
#[tokio::test]
async fn test_system_agents_visible_in_list() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    // Get the user's tenant from the tenant where they are the owner
    let all_tenants = resources.common.repos.tenants.get_all().await.unwrap();
    let user_tenant = all_tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .unwrap();
    let tenant_id = user_tenant.id;

    // Create a system agent directly in the database
    let agents_manager = &resources.common.repos.agents;
    let system_request = CreateSystemAgentRequest {
        title: "Platform Coach".to_owned(),
        description: Some("A system-wide coach".to_owned()),
        system_prompt: "You are a platform-wide fitness coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["system".to_owned()],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let system_coach = agents_manager
        .create_system_agent(user_id, tenant_id, &system_request)
        .await
        .unwrap();

    assert!(system_coach.is_system);

    // Generate a JWT token for the user
    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");

    // Create the agents router
    let router = build_agents_router::<ServerContext>().with_state(resources);

    // List agents via the API - should include the system agent
    let list_response = AxumTestRequest::get("/api/agents")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(list_response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = list_response.json();
    // Should have at least 1 agent (the system agent)
    assert!(!list.agents.is_empty());

    // Find the system agent in the response
    let found_system_coach = list.agents.iter().find(|c| c.title == "Platform Coach");
    assert!(
        found_system_coach.is_some(),
        "System coach should be visible in the list"
    );
    assert!(
        found_system_coach.unwrap().is_system,
        "Coach should be marked as system"
    );
}

/// E2E test: System agents should be retrievable by ID
#[tokio::test]
async fn test_get_system_agent_by_id() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    // Get the user's tenant from the tenant where they are the owner
    let all_tenants = resources.common.repos.tenants.get_all().await.unwrap();
    let user_tenant = all_tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .unwrap();
    let tenant_id = user_tenant.id;

    // Create a system agent
    let agents_manager = &resources.common.repos.agents;
    let system_request = CreateSystemAgentRequest {
        title: "Retrievable Coach".to_owned(),
        description: None,
        system_prompt: "You are a coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let system_coach = agents_manager
        .create_system_agent(user_id, tenant_id, &system_request)
        .await
        .unwrap();

    // Generate JWT token
    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");

    let router = build_agents_router::<ServerContext>().with_state(resources);

    // Get the system agent by ID via the API
    let get_response = AxumTestRequest::get(&format!("/api/agents/{}", system_coach.id))
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(get_response.status_code(), StatusCode::OK);

    let agent: AgentResponse = get_response.json();
    assert_eq!(agent.id, system_coach.id.to_string());
    assert_eq!(agent.title, "Retrievable Coach");
    assert!(agent.is_system);
}

// ============================================================================
// Hide/Show Agent E2E Tests
// ============================================================================

/// E2E test: Hidden agents appear when `include_hidden=true`
#[tokio::test]
async fn test_list_with_include_hidden() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, user) = create_test_user(&resources.agent.database).await.unwrap();

    // Get the user's tenant from the tenant where they are the owner
    let all_tenants = resources.common.repos.tenants.get_all().await.unwrap();
    let user_tenant = all_tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .unwrap();
    let tenant_id = user_tenant.id;

    // Create a system agent
    let agents_manager = &resources.common.repos.agents;
    let system_request = CreateSystemAgentRequest {
        title: "Hidden But Findable".to_owned(),
        description: None,
        system_prompt: "You are a coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let system_coach = agents_manager
        .create_system_agent(user_id, tenant_id, &system_request)
        .await
        .unwrap();

    // Hide the agent
    agents_manager
        .hide_agent(&system_coach.id.to_string(), user_id, tenant_id)
        .await
        .unwrap();

    // Generate JWT token
    let token = generate_test_token(&resources, &user).await;
    let auth_token = format!("Bearer {token}");

    let router = build_agents_router::<ServerContext>().with_state(resources);

    // List with include_hidden=true
    let list_response = AxumTestRequest::get("/api/agents?include_hidden=true")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(list_response.status_code(), StatusCode::OK);

    let list: ListAgentsResponse = list_response.json();
    // The hidden agent should appear when include_hidden=true
    let found_coach = list
        .agents
        .iter()
        .find(|c| c.title == "Hidden But Findable");
    assert!(
        found_coach.is_some(),
        "Hidden coach should appear when include_hidden=true"
    );
}

/// Regression: version-history reads must be owner/assigned/system-scoped, not
/// merely tenant-scoped. A different user WITHIN THE SAME TENANT must not be
/// able to read another user's private agent version history (which leaks the
/// agent's `system_prompt`/title/tags). The attacker's token is minted with the
/// VICTIM's `active_tenant_id`, so tenant scoping alone would let it through —
/// only the ownership check on the agent closes the IDOR.
#[tokio::test]
async fn test_version_reads_denied_for_non_owner_same_tenant() {
    let resources = create_test_server_resources().await.unwrap();

    // Victim (owner) and their tenant.
    let (owner_id, owner) = create_test_user(&resources.agent.database).await.unwrap();
    let owner_token = format!("Bearer {}", generate_test_token(&resources, &owner).await);
    let owner_tenant_id = resources
        .common
        .repos
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .map(|t| t.id)
        .expect("test user owns a tenant");
    let owner_tenant = Some(owner_tenant_id.to_string());

    // Attacker: a DIFFERENT user, but carrying the victim's active tenant in
    // their JWT so the request is same-tenant, different-user.
    let (_attacker_id, attacker) =
        create_test_user_with_email(&resources.agent.database, "attacker@example.com")
            .await
            .unwrap();
    let attacker_token = format!(
        "Bearer {}",
        resources
            .auth
            .auth_manager
            .generate_token_with_tenant(&attacker, &resources.auth.jwks_manager, owner_tenant)
            .unwrap()
    );

    // Owner creates a private agent.
    let agent = Seed {
        resources: Arc::clone(&resources),
        user_id: owner_id,
        tenant_id: owner_tenant_id,
    }
    .agent(json!({
        "title": "Private Coach",
        "system_prompt": "secret system prompt"
    }))
    .await;

    let router = build_agents_router::<ServerContext>().with_state(resources);

    // Owner updates it to produce version 1 (snapshot of the original state).
    let update = AxumTestRequest::put(&format!("/api/agents/{}", agent.id))
        .header("authorization", &owner_token)
        .json(&json!({
            "title": "Private Coach v2",
            "system_prompt": "secret system prompt v2"
        }))
        .send(router.clone())
        .await;
    assert_eq!(update.status_code(), StatusCode::OK);

    // Attacker (same tenant) must NOT list the version history.
    let attacker_list = AxumTestRequest::get(&format!("/api/agents/{}/versions", agent.id))
        .header("authorization", &attacker_token)
        .send(router.clone())
        .await;
    assert_eq!(
        attacker_list.status_code(),
        StatusCode::NOT_FOUND,
        "non-owner (same tenant) must not read version history"
    );

    // Nor diff a stored version against the current content.
    let attacker_diff = AxumTestRequest::get(&format!("/api/agents/{}/versions/1/diff", agent.id))
        .header("authorization", &attacker_token)
        .send(router.clone())
        .await;
    assert_eq!(
        attacker_diff.status_code(),
        StatusCode::NOT_FOUND,
        "non-owner (same tenant) must not diff version history"
    );

    // Positive control: the legitimate owner CAN read the history on the same URL.
    let owner_list = AxumTestRequest::get(&format!("/api/agents/{}/versions", agent.id))
        .header("authorization", &owner_token)
        .send(router)
        .await;
    assert_eq!(owner_list.status_code(), StatusCode::OK);
}

/// The web editor's version-history flow, end to end over the routes: two edits
/// write two snapshots, the diff compares a stored version with the CURRENT
/// content (which is never itself a stored version), and a revert snapshots the
/// content it replaces so the latest edit is not lost.
#[tokio::test]
async fn test_version_history_diff_against_current_and_lossless_revert() {
    let (router, auth, seed) = setup_test_environment().await;
    let agent = seed
        .agent(json!({ "title": "Tempo Coach", "system_prompt": "prompt one" }))
        .await;

    for (title, prompt) in [
        ("Tempo Coach B", "prompt two"),
        ("Tempo Coach C", "prompt three"),
    ] {
        let update = AxumTestRequest::put(&format!("/api/agents/{}", agent.id))
            .header("authorization", &auth)
            .json(&json!({ "title": title, "system_prompt": prompt }))
            .send(router.clone())
            .await;
        assert_eq!(update.status_code(), StatusCode::OK);
    }

    let list: serde_json::Value =
        AxumTestRequest::get(&format!("/api/agents/{}/versions", agent.id))
            .header("authorization", &auth)
            .send(router.clone())
            .await
            .assert_status(StatusCode::OK)
            .json();
    assert_eq!(list["current_version"], 2);
    assert_eq!(list["total"], 2);
    assert_eq!(list["versions"][0]["version"], 2);
    assert_eq!(
        list["versions"][0]["content_snapshot"]["title"],
        "Tempo Coach B"
    );
    assert_eq!(
        list["versions"][1]["content_snapshot"]["title"],
        "Tempo Coach"
    );

    // Version 1 against the live agent ("Tempo Coach C"), not against version 2.
    let diff: serde_json::Value =
        AxumTestRequest::get(&format!("/api/agents/{}/versions/1/diff", agent.id))
            .header("authorization", &auth)
            .send(router.clone())
            .await
            .assert_status(StatusCode::OK)
            .json();
    assert_eq!(diff["version"], 1);
    let changes = diff["changes"].as_array().unwrap();
    let title = changes.iter().find(|c| c["field"] == "title").unwrap();
    assert_eq!(title["old_value"], "Tempo Coach");
    assert_eq!(title["new_value"], "Tempo Coach C");
    let prompt = changes
        .iter()
        .find(|c| c["field"] == "system_prompt")
        .unwrap();
    assert_eq!(prompt["old_value"], "prompt one");
    assert_eq!(prompt["new_value"], "prompt three");

    let revert: serde_json::Value =
        AxumTestRequest::post(&format!("/api/agents/{}/versions/1/revert", agent.id))
            .header("authorization", &auth)
            .send(router.clone())
            .await
            .assert_status(StatusCode::OK)
            .json();
    assert_eq!(revert["agent"]["title"], "Tempo Coach");
    assert_eq!(revert["reverted_to_version"], 1);
    assert_eq!(revert["new_version"], 3);

    // Version 3 holds the edit the revert replaced, so it can be restored.
    let after: serde_json::Value =
        AxumTestRequest::get(&format!("/api/agents/{}/versions", agent.id))
            .header("authorization", &auth)
            .send(router.clone())
            .await
            .assert_status(StatusCode::OK)
            .json();
    assert_eq!(after["versions"][0]["version"], 3);
    assert_eq!(
        after["versions"][0]["content_snapshot"]["title"],
        "Tempo Coach C"
    );
    assert_eq!(
        after["versions"][0]["change_summary"],
        "Reverted to version 1"
    );

    // Diffing version 1 against the reverted agent now shows no content change.
    let clean: serde_json::Value =
        AxumTestRequest::get(&format!("/api/agents/{}/versions/1/diff", agent.id))
            .header("authorization", &auth)
            .send(router)
            .await
            .assert_status(StatusCode::OK)
            .json();
    assert_eq!(clean["changes"].as_array().unwrap().len(), 0);
}
