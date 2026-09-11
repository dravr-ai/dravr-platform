// ABOUTME: Unit tests for the agents database module
// ABOUTME: Tests CRUD operations, favorites, active agent, and multi-tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Test files: allow missing_docs (rustc lint) and unwrap (valid in tests per CLAUDE.md guidelines)
#![allow(missing_docs, clippy::unwrap_used)]

use chrono::Utc;
use pierre_core::field_update::FieldUpdate;
use pierre_core::models::agents::{
    AgentCategory, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest, ListAgentsFilter,
    UpdateAgentRequest,
};
use pierre_core::models::{CoachingPersona, Tenant, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils;
use uuid::Uuid;

/// A user row for the seeded fixture.
fn test_user(id: Uuid, email: &str) -> User {
    User {
        id,
        email: email.to_owned(),
        display_name: Some("Coaches Test".to_owned()),
        password_hash: "hash".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    }
}

/// Open a test database through the factory and seed the two users, two
/// tenants, and four memberships every test relies on.
async fn create_test_db() -> Database {
    let db = test_utils::create_test_db().await.unwrap();
    let repos = db.repositories();

    for (user_id, email) in [
        (test_user_id(), "test@example.com"),
        (other_user_id(), "other@example.com"),
    ] {
        repos
            .users
            .create(&test_user(user_id, email))
            .await
            .unwrap();
    }

    for (tenant_id, slug) in [
        (test_tenant(), "test-tenant"),
        (other_tenant(), "other-tenant"),
    ] {
        repos
            .tenants
            .create(&Tenant {
                id: tenant_id,
                name: format!("Tenant {slug}"),
                slug: slug.to_owned(),
                domain: None,
                plan: "starter".to_owned(),
                owner_user_id: test_user_id(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    // Memberships. Selecting an agent writes the membership row, so a user
    // with no membership in a tenant cannot select there — which is correct,
    // since agents are tenant-scoped, and is what the isolation tests below
    // exercise. Both users belong to both tenants so the isolation assertions
    // test the *selection*, not an accidental absence of membership.
    for (user_id, tenant_id) in [
        (test_user_id(), test_tenant()),
        (test_user_id(), other_tenant()),
        (other_user_id(), test_tenant()),
        (other_user_id(), other_tenant()),
    ] {
        repos
            .users
            .update_tenant_id(user_id, tenant_id)
            .await
            .unwrap();
    }

    db
}

fn test_user_id() -> Uuid {
    Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap()
}

fn other_user_id() -> Uuid {
    Uuid::parse_str("660e8400-e29b-41d4-a716-446655440000").unwrap()
}

fn test_tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa").unwrap())
}

fn other_tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb").unwrap())
}

// ============================================================================
// Create Tests
// ============================================================================

#[tokio::test]
async fn test_create_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Marathon Coach".to_owned(),
        description: Some("Helps with marathon training".to_owned()),
        system_prompt: "You are an expert marathon coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["running".to_owned(), "marathon".to_owned()],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    assert!(!agent.id.is_nil());
    assert_eq!(agent.user_id, test_user_id());
    assert_eq!(agent.tenant_id, test_tenant().to_string());
    assert_eq!(agent.title, "Marathon Coach");
    assert_eq!(
        agent.description,
        Some("Helps with marathon training".to_owned())
    );
    assert_eq!(agent.system_prompt, "You are an expert marathon coach.");
    assert_eq!(agent.category, AgentCategory::Training);
    assert_eq!(agent.tags, vec!["running", "marathon"]);
    // Preference fields (is_favorite, is_active, use_count, last_used_at) now live in coach_assignments
    // Token count should be estimated (~4 chars per token)
    assert!(agent.token_count > 0);
}

#[tokio::test]
async fn test_create_agent_minimal() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Simple Coach".to_owned(),
        description: None,
        system_prompt: "You help.".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    assert_eq!(agent.title, "Simple Coach");
    assert!(agent.description.is_none());
    assert_eq!(agent.category, AgentCategory::Custom);
    assert!(agent.tags.is_empty());
}

// ============================================================================
// Get Tests
// ============================================================================

#[tokio::test]
async fn test_get_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Test Coach".to_owned(),
        description: None,
        system_prompt: "Test prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let created = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let fetched = manager
        .get_by_id(&created.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, "Test Coach");
}

#[tokio::test]
async fn test_get_agent_not_found() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let result = manager
        .get_by_id("nonexistent-id", test_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_get_agent_wrong_user() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Private Coach".to_owned(),
        description: None,
        system_prompt: "Private prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let created = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Try to get with different user - should not find it
    let result = manager
        .get_by_id(&created.id.to_string(), other_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(result.is_none());
}

// ============================================================================
// List Tests
// ============================================================================

#[tokio::test]
async fn test_list_agents_empty() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let filter = ListAgentsFilter::default();
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert!(agents.is_empty());
}

#[tokio::test]
async fn test_list_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create multiple agents
    for i in 1..=3 {
        let request = CreateAgentRequest {
            title: format!("Coach {i}"),
            description: None,
            system_prompt: format!("Prompt {i}"),
            category: AgentCategory::Custom,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        manager
            .create(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
    }

    let filter = ListAgentsFilter::default();
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 3);
}

#[tokio::test]
async fn test_list_agents_by_category() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create agents with different categories
    let categories = [
        AgentCategory::Training,
        AgentCategory::Nutrition,
        AgentCategory::Training,
    ];

    for (i, category) in categories.iter().enumerate() {
        let request = CreateAgentRequest {
            title: format!("Coach {}", i + 1),
            description: None,
            system_prompt: format!("Prompt {}", i + 1),
            category: *category,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        manager
            .create(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
    }

    // Filter by Training category
    let filter = ListAgentsFilter {
        category: Some(AgentCategory::Training),
        ..Default::default()
    };
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 2);
    for item in &agents {
        assert_eq!(item.agent.category, AgentCategory::Training);
    }
}

#[tokio::test]
async fn test_list_agents_favorites_only() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create agents
    let mut agent_ids = Vec::new();
    for i in 1..=3 {
        let request = CreateAgentRequest {
            title: format!("Coach {i}"),
            description: None,
            system_prompt: format!("Prompt {i}"),
            category: AgentCategory::Custom,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        let agent = manager
            .create(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
        agent_ids.push(agent.id.to_string());
    }

    // Mark first agent as favorite
    manager
        .toggle_favorite(&agent_ids[0], test_user_id(), test_tenant())
        .await
        .unwrap();

    // Filter favorites only
    let filter = ListAgentsFilter {
        favorites_only: true,
        ..Default::default()
    };
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 1);
    assert!(agents[0].is_favorite);
}

#[tokio::test]
async fn test_list_agents_with_pagination() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create 5 agents
    for i in 1..=5 {
        let request = CreateAgentRequest {
            title: format!("Coach {i}"),
            description: None,
            system_prompt: format!("Prompt {i}"),
            category: AgentCategory::Custom,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        manager
            .create(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
    }

    // Get first page (limit 2)
    let filter = ListAgentsFilter {
        limit: Some(2),
        offset: Some(0),
        ..Default::default()
    };
    let page1 = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);

    // Get second page
    let filter = ListAgentsFilter {
        limit: Some(2),
        offset: Some(2),
        ..Default::default()
    };
    let page2 = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);

    // Get third page (only 1 remaining)
    let filter = ListAgentsFilter {
        limit: Some(2),
        offset: Some(4),
        ..Default::default()
    };
    let page3 = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(page3.len(), 1);
}

#[tokio::test]
async fn test_list_agents_user_isolation() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create agent for user 1
    let request = CreateAgentRequest {
        title: "User 1 Coach".to_owned(),
        description: None,
        system_prompt: "Prompt 1".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Create agent for user 2
    let request = CreateAgentRequest {
        title: "User 2 Coach".to_owned(),
        description: None,
        system_prompt: "Prompt 2".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    manager
        .create(other_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // User 1 should only see their agent
    let filter = ListAgentsFilter::default();
    let user1_coaches = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(user1_coaches.len(), 1);
    assert_eq!(user1_coaches[0].agent.title, "User 1 Coach");

    // User 2 should only see their agent
    let user2_coaches = manager
        .list(other_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(user2_coaches.len(), 1);
    assert_eq!(user2_coaches[0].agent.title, "User 2 Coach");
}

// ============================================================================
// Update Tests
// ============================================================================

#[tokio::test]
async fn test_update_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Original Title".to_owned(),
        description: Some("Original description".to_owned()),
        system_prompt: "Original prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec!["tag1".to_owned()],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let update = UpdateAgentRequest {
        title: Some("Updated Title".to_owned()),
        description: Some("Updated description".to_owned()),
        system_prompt: Some("Updated prompt".to_owned()),
        category: Some(AgentCategory::Training),
        tags: Some(vec!["tag2".to_owned(), "tag3".to_owned()]),
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Keep,
    };

    let updated = manager
        .update(
            &agent.id.to_string(),
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap();

    assert!(updated.is_some());
    let updated = updated.unwrap();
    assert_eq!(updated.title, "Updated Title");
    assert_eq!(updated.description, Some("Updated description".to_owned()));
    assert_eq!(updated.system_prompt, "Updated prompt");
    assert_eq!(updated.category, AgentCategory::Training);
    assert_eq!(updated.tags, vec!["tag2", "tag3"]);
}

#[tokio::test]
async fn test_update_agent_partial() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Original Title".to_owned(),
        description: Some("Original description".to_owned()),
        system_prompt: "Original prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["tag1".to_owned()],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Only update title
    let update = UpdateAgentRequest {
        title: Some("New Title".to_owned()),
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Keep,
    };

    let updated = manager
        .update(
            &agent.id.to_string(),
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(updated.title, "New Title");
    assert_eq!(updated.description, Some("Original description".to_owned()));
    assert_eq!(updated.system_prompt, "Original prompt");
    assert_eq!(updated.category, AgentCategory::Training);
}

#[tokio::test]
async fn test_update_agent_not_found() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let update = UpdateAgentRequest {
        title: Some("New Title".to_owned()),
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Keep,
    };

    let result = manager
        .update(
            "nonexistent-id",
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap();

    assert!(result.is_none());
}

// ============================================================================
// Delete Tests
// ============================================================================

#[tokio::test]
async fn test_delete_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "To Delete".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let deleted = manager
        .delete(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(deleted);

    // Verify it's gone
    let result = manager
        .get_by_id(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_delete_agent_not_found() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let deleted = manager
        .delete("nonexistent-id", test_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(!deleted);
}

#[tokio::test]
async fn test_delete_agent_wrong_user() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Private Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Try to delete with different user
    let deleted = manager
        .delete(&agent.id.to_string(), other_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(!deleted);

    // Verify it still exists for original user
    let result = manager
        .get_by_id(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(result.is_some());
}

// ============================================================================
// Favorite Tests
// ============================================================================

#[tokio::test]
async fn test_toggle_favorite() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Test Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    // Preference fields live in coach_assignments, not Agent struct

    // Toggle to favorite
    let is_favorite = manager
        .toggle_favorite(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert_eq!(is_favorite, Some(true));

    // Verify via list with favorites filter
    let filter = ListAgentsFilter {
        favorites_only: true,
        ..Default::default()
    };
    let favorites = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(favorites.len(), 1);
    assert!(favorites[0].is_favorite);

    // Toggle back
    let is_favorite = manager
        .toggle_favorite(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert_eq!(is_favorite, Some(false));

    // Verify no favorites
    let favorites = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert!(favorites.is_empty());
}

#[tokio::test]
async fn test_toggle_favorite_not_found() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let result = manager
        .toggle_favorite("nonexistent-id", test_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(result.is_none());
}

// ============================================================================
// Active Agent Tests
// ============================================================================

#[tokio::test]
async fn test_activate_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Active Coach".to_owned(),
        description: None,
        system_prompt: "Active prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    // Preference fields live in coach_assignments, not Agent struct

    // Activate
    let activated = manager
        .activate_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(activated.is_some());

    // Verify via get_active_agent
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_some());
    assert_eq!(active.unwrap().id, agent.id);
}

#[tokio::test]
async fn test_activate_agent_deactivates_others() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create two agents
    let request1 = CreateAgentRequest {
        title: "Coach 1".to_owned(),
        description: None,
        system_prompt: "Prompt 1".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let coach1 = manager
        .create(test_user_id(), test_tenant(), &request1)
        .await
        .unwrap();

    let request2 = CreateAgentRequest {
        title: "Coach 2".to_owned(),
        description: None,
        system_prompt: "Prompt 2".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let coach2 = manager
        .create(test_user_id(), test_tenant(), &request2)
        .await
        .unwrap();

    // Activate first agent
    manager
        .activate_agent(&coach1.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Verify first is active via get_active_agent
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_some());
    assert_eq!(active.unwrap().id, coach1.id);

    // Activate second agent
    manager
        .activate_agent(&coach2.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Verify second is now active (first was deactivated)
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_some());
    assert_eq!(active.unwrap().id, coach2.id);
}

#[tokio::test]
async fn test_deactivate_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Test Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Activate
    manager
        .activate_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Deactivate
    let deactivated = manager
        .deactivate_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(deactivated);

    // Verify no active agent
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_none());
}

#[tokio::test]
async fn test_deactivate_when_none_active() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Deactivate when nothing is active
    let deactivated = manager
        .deactivate_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(!deactivated);
}

#[tokio::test]
async fn test_get_active_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // No active agent initially
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_none());

    // Create and activate an agent
    let request = CreateAgentRequest {
        title: "Active Coach".to_owned(),
        description: None,
        system_prompt: "Active prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .activate_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Now should have active agent
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_some());
    let active = active.unwrap();
    assert_eq!(active.title, "Active Coach");
}

#[tokio::test]
async fn test_active_agent_user_isolation() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create agent for user 1 and activate
    let request = CreateAgentRequest {
        title: "User 1 Coach".to_owned(),
        description: None,
        system_prompt: "Prompt 1".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    manager
        .activate_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // User 1 should see active agent
    let active = manager
        .get_active_agent(test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_some());

    // User 2 should not see active agent
    let active = manager
        .get_active_agent(other_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(active.is_none());
}

// ============================================================================
// Usage Recording Tests
// ============================================================================

#[tokio::test]
async fn test_record_usage() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Test Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    // Preference fields live in coach_assignments, not Agent struct

    // Verify initial state via list
    let filter = ListAgentsFilter::default();
    let items = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    let item = items.iter().find(|i| i.agent.id == agent.id).unwrap();
    assert_eq!(item.use_count, 0);
    assert!(item.last_used_at.is_none());

    // Record usage
    let recorded = manager
        .record_usage(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(recorded);

    // Verify via list
    let items = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    let item = items.iter().find(|i| i.agent.id == agent.id).unwrap();
    assert_eq!(item.use_count, 1);
    assert!(item.last_used_at.is_some());

    // Record again
    manager
        .record_usage(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    let items = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    let item = items.iter().find(|i| i.agent.id == agent.id).unwrap();
    assert_eq!(item.use_count, 2);
}

// Regression for the 2026-05-07 admin audit: every system agent was stuck at
// "0 uses" because record_usage strictly required the caller's tenant to match
// the agent's pinned tenant. System agents live in the seed tenant but are
// exposed to every tenant via the catalog, so chatting with one from any other
// tenant silently skipped the use_count bump. The fix accepts is_system = TRUE
// unconditionally; this test pins that behaviour.
#[tokio::test]
async fn test_record_usage_for_system_agent_from_other_tenant() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateSystemAgentRequest {
        title: "Cross-tenant System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    assert!(agent.is_system);
    assert_eq!(agent.tenant_id, test_tenant().to_string());

    // A user from a *different* tenant chats with this system agent. Before the
    // fix, record_usage returned Ok(false) and coach_assignments stayed empty.
    let recorded = manager
        .record_usage(&agent.id.to_string(), other_user_id(), other_tenant())
        .await
        .unwrap();
    assert!(
        recorded,
        "system agents must accept use_count bumps from non-pinning tenants"
    );

    // Bumping a second time confirms the assignment row was created and the
    // counter increments rather than being recreated.
    manager
        .record_usage(&agent.id.to_string(), other_user_id(), other_tenant())
        .await
        .unwrap();

    let filter = ListAgentsFilter::default();
    let items = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();
    // System agent must surface in cross-tenant listing.
    let item = items.iter().find(|i| i.agent.id == agent.id).unwrap();
    assert_eq!(item.use_count, 2);
    assert!(item.last_used_at.is_some());
}

// Companion regression for activate_agent: non-seed-tenant users must be able
// to set a system agent as their active default. Audit (2026-05-07) traced the
// same tenant-strict select pattern across record_usage, toggle_favorite, and
// activate_agent — fixing only one would leave silent failures elsewhere.
#[tokio::test]
async fn test_activate_system_agent_from_other_tenant() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateSystemAgentRequest {
        title: "Activate-me Cross-tenant System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let activated = manager
        .activate_agent(&agent.id.to_string(), other_user_id(), other_tenant())
        .await
        .unwrap();
    assert!(
        activated.is_some(),
        "system agents must accept activation from non-pinning tenants"
    );
    assert_eq!(activated.unwrap().id, agent.id);
}

// ============================================================================
// Search Tests
// ============================================================================

#[tokio::test]
async fn test_search_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create agents with searchable content
    let requests = [
        CreateAgentRequest {
            title: "Marathon Coach".to_owned(),
            description: Some("Helps with marathon training".to_owned()),
            system_prompt: "You are a marathon expert".to_owned(),
            category: AgentCategory::Training,
            tags: vec!["running".to_owned(), "marathon".to_owned()],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        },
        CreateAgentRequest {
            title: "Nutrition Advisor".to_owned(),
            description: Some("Provides nutrition guidance".to_owned()),
            system_prompt: "You are a nutrition expert".to_owned(),
            category: AgentCategory::Nutrition,
            tags: vec!["diet".to_owned(), "nutrition".to_owned()],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        },
        CreateAgentRequest {
            title: "Recovery Coach".to_owned(),
            description: Some("Specializes in recovery and rest".to_owned()),
            system_prompt: "You help with recovery".to_owned(),
            category: AgentCategory::Recovery,
            tags: vec!["rest".to_owned(), "recovery".to_owned()],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        },
    ];

    for request in &requests {
        manager
            .create(test_user_id(), test_tenant(), request)
            .await
            .unwrap();
    }

    // Search by title
    let results = manager
        .search(test_user_id(), test_tenant(), "Marathon", None, None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Marathon Coach");

    // Search by description
    let results = manager
        .search(test_user_id(), test_tenant(), "nutrition", None, None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Nutrition Advisor");

    // Search by tags
    let results = manager
        .search(test_user_id(), test_tenant(), "running", None, None)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Marathon Coach");

    // Search with no results
    let results = manager
        .search(test_user_id(), test_tenant(), "swimming", None, None)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_search_agents_with_limit() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create multiple agents with "Coach" in title
    for i in 1..=5 {
        let request = CreateAgentRequest {
            title: format!("Coach {i}"),
            description: None,
            system_prompt: format!("Prompt {i}"),
            category: AgentCategory::Custom,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        manager
            .create(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
    }

    // Search with limit
    let results = manager
        .search(test_user_id(), test_tenant(), "Coach", Some(2), None)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
}

// ============================================================================
// Count Tests
// ============================================================================

#[tokio::test]
async fn test_count_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Initially zero
    let count = manager.count(test_user_id(), test_tenant()).await.unwrap();
    assert_eq!(count, 0);

    // Create agents
    for i in 1..=3 {
        let request = CreateAgentRequest {
            title: format!("Coach {i}"),
            description: None,
            system_prompt: format!("Prompt {i}"),
            category: AgentCategory::Custom,
            tags: vec![],
            sample_prompts: vec![],
            startup_query: None,
            data_requirements: None,
            purpose: None,
            when_to_use: None,
            instructions: None,
            example_inputs: None,
            example_outputs: None,
            success_criteria: None,
            max_tool_iterations: None,
        };
        manager
            .create(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
    }

    let count = manager.count(test_user_id(), test_tenant()).await.unwrap();
    assert_eq!(count, 3);
}

// ============================================================================
// Category Tests
// ============================================================================

#[test]
fn test_agent_category_parsing() {
    // Lowercase parsing
    assert_eq!(AgentCategory::parse("training"), AgentCategory::Training);
    assert_eq!(AgentCategory::parse("nutrition"), AgentCategory::Nutrition);
    assert_eq!(AgentCategory::parse("recovery"), AgentCategory::Recovery);
    assert_eq!(AgentCategory::parse("recipes"), AgentCategory::Recipes);
    assert_eq!(AgentCategory::parse("mobility"), AgentCategory::Mobility);
    assert_eq!(AgentCategory::parse("custom"), AgentCategory::Custom);

    // Case-insensitive parsing (frontend sends capitalized values)
    assert_eq!(AgentCategory::parse("Training"), AgentCategory::Training);
    assert_eq!(AgentCategory::parse("Nutrition"), AgentCategory::Nutrition);
    assert_eq!(AgentCategory::parse("Recovery"), AgentCategory::Recovery);
    assert_eq!(AgentCategory::parse("Recipes"), AgentCategory::Recipes);
    assert_eq!(AgentCategory::parse("Mobility"), AgentCategory::Mobility);
    assert_eq!(AgentCategory::parse("Custom"), AgentCategory::Custom);

    // Mixed case
    assert_eq!(AgentCategory::parse("TRAINING"), AgentCategory::Training);
    assert_eq!(AgentCategory::parse("TraInInG"), AgentCategory::Training);

    // Unknown values default to Custom
    assert_eq!(AgentCategory::parse("unknown"), AgentCategory::Custom);
    assert_eq!(AgentCategory::parse(""), AgentCategory::Custom);
}

#[test]
fn test_agent_category_as_str() {
    assert_eq!(AgentCategory::Training.as_str(), "training");
    assert_eq!(AgentCategory::Nutrition.as_str(), "nutrition");
    assert_eq!(AgentCategory::Recovery.as_str(), "recovery");
    assert_eq!(AgentCategory::Recipes.as_str(), "recipes");
    assert_eq!(AgentCategory::Mobility.as_str(), "mobility");
    assert_eq!(AgentCategory::Custom.as_str(), "custom");
}

#[test]
fn test_agent_category_round_trip() {
    // Verify that as_str -> parse round-trips correctly for all variants
    let categories = [
        AgentCategory::Training,
        AgentCategory::Nutrition,
        AgentCategory::Recovery,
        AgentCategory::Recipes,
        AgentCategory::Mobility,
        AgentCategory::Custom,
    ];

    for category in categories {
        let serialized = category.as_str();
        let deserialized = AgentCategory::parse(serialized);
        assert_eq!(category, deserialized, "Round-trip failed for {serialized}");
    }
}

#[test]
fn test_agent_category_serde_serialization() {
    // Test serde serialization produces snake_case values
    let training = AgentCategory::Training;
    let json = serde_json::to_string(&training).unwrap();
    assert_eq!(json, "\"training\"");

    let mobility = AgentCategory::Mobility;
    let json = serde_json::to_string(&mobility).unwrap();
    assert_eq!(json, "\"mobility\"");

    let custom = AgentCategory::Custom;
    let json = serde_json::to_string(&custom).unwrap();
    assert_eq!(json, "\"custom\"");
}

#[test]
fn test_agent_category_serde_deserialization() {
    // Test serde deserialization handles various cases
    let training: AgentCategory = serde_json::from_str("\"training\"").unwrap();
    assert_eq!(training, AgentCategory::Training);

    let mobility: AgentCategory = serde_json::from_str("\"mobility\"").unwrap();
    assert_eq!(mobility, AgentCategory::Mobility);

    // Note: serde uses rename_all = "snake_case", so it expects lowercase
    // Frontend may send capitalized values through the API which gets parsed via parse()
}

// ============================================================================
// System Agent Tests
// ============================================================================

#[tokio::test]
async fn test_create_system_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateSystemAgentRequest {
        title: "Pierre Default Coach".to_owned(),
        description: Some("The official Pierre fitness coach".to_owned()),
        system_prompt: "You are Pierre, an expert fitness coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["official".to_owned(), "default".to_owned()],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    assert!(!agent.id.is_nil());
    assert_eq!(agent.tenant_id, test_tenant().to_string());
    assert_eq!(agent.title, "Pierre Default Coach");
    assert!(agent.is_system);
    assert_eq!(agent.visibility, AgentVisibility::Tenant);
}

#[tokio::test]
async fn test_list_system_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create two system agents
    for i in 1..=2 {
        let request = CreateSystemAgentRequest {
            title: format!("System Coach {i}"),
            description: None,
            system_prompt: format!("System prompt {i}"),
            category: AgentCategory::Training,
            tags: vec![],
            visibility: AgentVisibility::Tenant,
            sample_prompts: vec![],
        };
        manager
            .create_system_agent(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
    }

    let agents = manager.list_system_agents(test_tenant()).await.unwrap();
    assert_eq!(agents.len(), 2);
    assert!(agents.iter().all(|c| c.is_system));
}

#[tokio::test]
async fn test_get_system_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: Some("System description".to_owned()),
        system_prompt: "System prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let created = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let fetched = manager
        .get_system_agent(&created.id.to_string(), test_tenant())
        .await
        .unwrap();

    assert!(fetched.is_some());
    let fetched = fetched.unwrap();
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.title, "System Coach");
    assert!(fetched.is_system);
}

#[tokio::test]
async fn test_update_system_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateSystemAgentRequest {
        title: "Original System Coach".to_owned(),
        description: Some("Original description".to_owned()),
        system_prompt: "Original prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["original".to_owned()],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let update = UpdateAgentRequest {
        title: Some("Updated System Coach".to_owned()),
        description: Some("Updated description".to_owned()),
        system_prompt: Some("Updated prompt".to_owned()),
        category: Some(AgentCategory::Nutrition),
        tags: Some(vec!["updated".to_owned()]),
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Keep,
    };

    let updated = manager
        .update_system_agent(&agent.id.to_string(), test_tenant(), &update)
        .await
        .unwrap();

    assert!(updated.is_some());
    let updated = updated.unwrap();
    assert_eq!(updated.title, "Updated System Coach");
    assert_eq!(updated.description, Some("Updated description".to_owned()));
    assert_eq!(updated.category, AgentCategory::Nutrition);
    assert!(updated.is_system);
}

#[tokio::test]
async fn test_delete_system_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateSystemAgentRequest {
        title: "To Delete".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let deleted = manager
        .delete_system_agent(&agent.id.to_string(), test_tenant())
        .await
        .unwrap();

    assert!(deleted);

    // Verify it's gone
    let result = manager
        .get_system_agent(&agent.id.to_string(), test_tenant())
        .await
        .unwrap();
    assert!(result.is_none());
}

// ============================================================================
// Agent Assignment Tests
// ============================================================================

#[tokio::test]
async fn test_assign_agent_to_user() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "System prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Assign to user
    let assigned = manager
        .assign_agent(&agent.id.to_string(), other_user_id(), test_user_id())
        .await
        .unwrap();

    assert!(assigned);

    // Verify assignment
    let assignments = manager
        .list_assignments(&agent.id.to_string())
        .await
        .unwrap();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].user_id, other_user_id().to_string());
}

#[tokio::test]
async fn test_unassign_agent_from_user() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create and assign
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .assign_agent(&agent.id.to_string(), other_user_id(), test_user_id())
        .await
        .unwrap();

    // Unassign
    let unassigned = manager
        .unassign_agent(&agent.id.to_string(), other_user_id())
        .await
        .unwrap();

    assert!(unassigned);

    // Verify unassignment
    let assignments = manager
        .list_assignments(&agent.id.to_string())
        .await
        .unwrap();
    assert!(assignments.is_empty());
}

#[tokio::test]
async fn test_list_assignments() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create system agent
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Assign to both users
    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();
    manager
        .assign_agent(&agent.id.to_string(), other_user_id(), test_user_id())
        .await
        .unwrap();

    let assignments = manager
        .list_assignments(&agent.id.to_string())
        .await
        .unwrap();
    assert_eq!(assignments.len(), 2);
}

#[tokio::test]
async fn test_list_agents_includes_assigned_system_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a personal agent for user 1
    let personal_request = CreateAgentRequest {
        title: "Personal Coach".to_owned(),
        description: None,
        system_prompt: "Personal prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    manager
        .create(test_user_id(), test_tenant(), &personal_request)
        .await
        .unwrap();

    // Create a system agent and assign to user 1
    let system_request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "System prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let system_coach = manager
        .create_system_agent(test_user_id(), test_tenant(), &system_request)
        .await
        .unwrap();

    manager
        .assign_agent(&system_coach.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();

    // User 1 should see both agents
    let filter = ListAgentsFilter::default();
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 2);

    // Both should be assigned: personal agent via self-assignment from create(),
    // system agent via explicit assign_agent() call
    let assigned_count = agents.iter().filter(|c| c.is_assigned).count();
    assert_eq!(assigned_count, 2);

    // Verify the system agent is in the list and assigned
    let system_coach = agents
        .iter()
        .find(|c| c.agent.title == "System Coach")
        .unwrap();
    assert!(system_coach.is_assigned);
}

// ============================================================================
// Hide/Show Agent Tests
// ============================================================================

#[tokio::test]
async fn test_hide_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent and assign it
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();

    // Hide the agent
    let hidden = manager
        .hide_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(hidden);
}

#[tokio::test]
async fn test_hide_agent_not_found() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Try to hide non-existent agent - should return error
    let result = manager
        .hide_agent("nonexistent-id", test_user_id(), test_tenant())
        .await;

    assert!(result.is_err());
}

/// An agent assigned to the user but owned by ANOTHER tenant must answer
/// exactly like a nonexistent id — the pre-fix behavior (tenant ignored)
/// made `hide_agent` a cross-tenant existence oracle for agent ids.
#[tokio::test]
async fn test_hide_agent_in_another_tenant_reads_as_not_hideable() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Tenant-bound Coach".to_owned(),
        description: None,
        system_prompt: "You help.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    let agent = manager
        .create(other_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), other_user_id())
        .await
        .unwrap();

    // Same user, same assignment — wrong tenant: refused, with the SAME error
    // shape as a nonexistent id, so the response confirms nothing.
    let foreign = manager
        .hide_agent(&agent.id.to_string(), test_user_id(), other_tenant())
        .await;
    let missing = manager
        .hide_agent("nonexistent-id", test_user_id(), other_tenant())
        .await;
    assert!(foreign.is_err());
    assert_eq!(
        foreign.unwrap_err().to_string(),
        missing.unwrap_err().to_string(),
        "a foreign tenant's coach id must be indistinguishable from a missing one"
    );

    // The right tenant can hide it.
    let hidden = manager
        .hide_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();
    assert!(hidden);
}

#[tokio::test]
async fn test_show_agent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent and assign it
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();

    // Hide first
    manager
        .hide_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Then show
    let shown = manager
        .show_agent(&agent.id.to_string(), test_user_id())
        .await
        .unwrap();

    assert!(shown);
}

#[tokio::test]
async fn test_list_hidden_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create two system agents
    let mut agent_ids = Vec::new();
    for i in 1..=2 {
        let request = CreateSystemAgentRequest {
            title: format!("System Coach {i}"),
            description: None,
            system_prompt: format!("Prompt {i}"),
            category: AgentCategory::Training,
            tags: vec![],
            visibility: AgentVisibility::Tenant,
            sample_prompts: vec![],
        };
        let agent = manager
            .create_system_agent(test_user_id(), test_tenant(), &request)
            .await
            .unwrap();
        agent_ids.push(agent.id.to_string());
    }

    // Assign both to user
    for id in &agent_ids {
        manager
            .assign_agent(id, test_user_id(), test_user_id())
            .await
            .unwrap();
    }

    // Hide only the first one
    manager
        .hide_agent(&agent_ids[0], test_user_id(), test_tenant())
        .await
        .unwrap();

    // List hidden agents
    let hidden = manager
        .list_hidden_agents(test_user_id(), test_tenant())
        .await
        .unwrap();

    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].id.to_string(), agent_ids[0]);
    assert_eq!(hidden[0].title, "System Coach 1");
}

#[tokio::test]
async fn test_hidden_agent_excluded_from_list() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent and assign it
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();

    // Should see 1 agent
    let filter = ListAgentsFilter::default();
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(agents.len(), 1);

    // Hide the agent
    manager
        .hide_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Should see 0 agents now
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(agents.len(), 0);
}

#[tokio::test]
async fn test_unhidden_agent_appears_in_list() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent and assign it
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();

    // Hide the agent
    manager
        .hide_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // Should see 0 agents
    let filter = ListAgentsFilter::default();
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(agents.len(), 0);

    // Unhide the agent
    manager
        .show_agent(&agent.id.to_string(), test_user_id())
        .await
        .unwrap();

    // Should see 1 agent again
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(agents.len(), 1);
}

#[tokio::test]
async fn test_hide_agent_user_isolation() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent and assign to both users
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    let agent = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    manager
        .assign_agent(&agent.id.to_string(), test_user_id(), test_user_id())
        .await
        .unwrap();
    manager
        .assign_agent(&agent.id.to_string(), other_user_id(), test_user_id())
        .await
        .unwrap();

    // User 1 hides the agent
    manager
        .hide_agent(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap();

    // User 1 should not see the agent
    let filter = ListAgentsFilter::default();
    let user1_coaches = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(user1_coaches.len(), 0);

    // User 2 should still see the agent
    let user2_coaches = manager
        .list(other_user_id(), test_tenant(), &filter)
        .await
        .unwrap();
    assert_eq!(user2_coaches.len(), 1);
}

// ============================================================================
// Cross-Tenant System Agent Visibility Tests
// ============================================================================

/// System agents with `is_system=1` should be visible to users from ANY tenant
/// when `include_system` filter is enabled, regardless of the tenant they were created in.
#[tokio::test]
async fn test_system_agent_visible_across_tenants() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent in test_tenant() (tenant A)
    let request = CreateSystemAgentRequest {
        title: "Global System Coach".to_owned(),
        description: Some("Visible to all tenants".to_owned()),
        system_prompt: "You are a globally available coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["global".to_owned()],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let system_coach = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    assert!(system_coach.is_system);
    assert_eq!(system_coach.tenant_id, test_tenant().to_string());

    // User from other_tenant() (tenant B) should see the system agent
    // when include_system filter is enabled
    let filter = ListAgentsFilter {
        include_system: true,
        ..Default::default()
    };

    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();

    // Should find the system agent even though user is from a different tenant
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent.title, "Global System Coach");
    assert!(agents[0].agent.is_system);
}

/// When `include_system` is false, system agents from other tenants should NOT be visible
#[tokio::test]
async fn test_system_agent_hidden_when_include_system_false() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent in test_tenant()
    let request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // User from other_tenant() should NOT see the system agent
    // when include_system is false (default)
    let filter = ListAgentsFilter {
        include_system: false,
        ..Default::default()
    };

    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();

    // Should not find any agents - no personal agents and system agents excluded
    assert!(agents.is_empty());
}

/// Multiple system agents from different tenants should all be visible
/// to users from any tenant when `include_system` is enabled
#[tokio::test]
async fn test_multiple_system_agents_visible_across_tenants() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create system agent in test_tenant()
    let request1 = CreateSystemAgentRequest {
        title: "System Coach From Tenant A".to_owned(),
        description: None,
        system_prompt: "Prompt A".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    manager
        .create_system_agent(test_user_id(), test_tenant(), &request1)
        .await
        .unwrap();

    // Create system agent in other_tenant()
    let request2 = CreateSystemAgentRequest {
        title: "System Coach From Tenant B".to_owned(),
        description: None,
        system_prompt: "Prompt B".to_owned(),
        category: AgentCategory::Nutrition,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    manager
        .create_system_agent(other_user_id(), other_tenant(), &request2)
        .await
        .unwrap();

    // User from test_tenant() should see both system agents
    let filter = ListAgentsFilter {
        include_system: true,
        ..Default::default()
    };

    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 2);
    let titles: Vec<&str> = agents.iter().map(|c| c.agent.title.as_str()).collect();
    assert!(titles.contains(&"System Coach From Tenant A"));
    assert!(titles.contains(&"System Coach From Tenant B"));

    // User from other_tenant() should also see both system agents
    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 2);
}

/// Personal agents should remain tenant-isolated even when system agents are visible
#[tokio::test]
async fn test_personal_agents_remain_isolated_with_system_agents() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a personal agent in test_tenant()
    let personal_request = CreateAgentRequest {
        title: "Personal Coach".to_owned(),
        description: None,
        system_prompt: "Personal prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };
    manager
        .create(test_user_id(), test_tenant(), &personal_request)
        .await
        .unwrap();

    // Create a system agent in test_tenant()
    let system_request = CreateSystemAgentRequest {
        title: "System Coach".to_owned(),
        description: None,
        system_prompt: "System prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };
    manager
        .create_system_agent(test_user_id(), test_tenant(), &system_request)
        .await
        .unwrap();

    // User from other_tenant() with include_system should see ONLY the system agent
    // NOT the personal agent from test_tenant()
    let filter = ListAgentsFilter {
        include_system: true,
        ..Default::default()
    };

    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent.title, "System Coach");
    assert!(agents[0].agent.is_system);

    // User from test_tenant() should see both their personal agent and the system agent
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 2);
}

/// System agents can be hidden by users from ANY tenant, not just the tenant that created them.
/// This is the expected behavior because system agents are globally visible.
#[tokio::test]
async fn test_hide_system_agent_cross_tenant() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent in test_tenant() (tenant A)
    let request = CreateSystemAgentRequest {
        title: "Global System Coach".to_owned(),
        description: None,
        system_prompt: "You are a globally available coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let system_coach = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    assert!(system_coach.is_system);
    assert_eq!(system_coach.tenant_id, test_tenant().to_string());

    // User from other_tenant() (tenant B) should be able to hide this system agent
    // Even though the agent was created by test_tenant()
    let hidden = manager
        .hide_agent(&system_coach.id.to_string(), other_user_id(), test_tenant())
        .await
        .unwrap();

    assert!(hidden);

    // Verify the agent is hidden for other_user
    let filter = ListAgentsFilter {
        include_system: true,
        include_hidden: false,
        ..Default::default()
    };

    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();

    // Should NOT see the system agent (it's hidden for this user)
    assert!(agents.is_empty());

    // But the original tenant user should still see it
    let agents = manager
        .list(test_user_id(), test_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent.title, "Global System Coach");
}

/// Users can show (unhide) system agents from other tenants
#[tokio::test]
async fn test_show_system_agent_cross_tenant() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent in test_tenant()
    let request = CreateSystemAgentRequest {
        title: "Global System Coach".to_owned(),
        description: None,
        system_prompt: "Prompt".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        visibility: AgentVisibility::Tenant,
        sample_prompts: vec![],
    };

    let system_coach = manager
        .create_system_agent(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // User from other_tenant() hides the agent
    manager
        .hide_agent(&system_coach.id.to_string(), other_user_id(), test_tenant())
        .await
        .unwrap();

    // Verify it's hidden
    let filter = ListAgentsFilter {
        include_system: true,
        include_hidden: false,
        ..Default::default()
    };

    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();
    assert!(agents.is_empty());

    // Now show (unhide) the agent
    let shown = manager
        .show_agent(&system_coach.id.to_string(), other_user_id())
        .await
        .unwrap();

    assert!(shown);

    // Should now see the agent again
    let agents = manager
        .list(other_user_id(), other_tenant(), &filter)
        .await
        .unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent.title, "Global System Coach");
}

// ============================================================================
// Structured Agent Section Tests
// ============================================================================

#[tokio::test]
async fn test_create_agent_with_structured_fields() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Structured Coach".to_owned(),
        description: Some("A coach with all structured sections".to_owned()),
        system_prompt: "Fallback prompt for legacy clients".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["structured".to_owned()],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: Some("Help athletes build a periodized marathon training plan.".to_owned()),
        when_to_use: Some("Use when preparing for a marathon 12+ weeks out.".to_owned()),
        instructions: Some("You are an expert marathon coach. Build a plan based on the athlete's current fitness level, goal time, and available training days.".to_owned()),
        example_inputs: Some("I want to run a sub-3:30 marathon in 16 weeks. I can train 5 days/week.".to_owned()),
        example_outputs: Some("Provide a week-by-week breakdown with daily workouts, paces, and recovery notes.".to_owned()),
        success_criteria: Some("The plan should be progressive, include tapering, and respect rest days.".to_owned()),
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Verify structured fields are stored
    assert_eq!(
        agent.purpose,
        Some("Help athletes build a periodized marathon training plan.".to_owned())
    );
    assert_eq!(
        agent.when_to_use,
        Some("Use when preparing for a marathon 12+ weeks out.".to_owned())
    );
    assert_eq!(
        agent.instructions,
        Some("You are an expert marathon coach. Build a plan based on the athlete's current fitness level, goal time, and available training days.".to_owned())
    );
    assert_eq!(
        agent.example_inputs,
        Some("I want to run a sub-3:30 marathon in 16 weeks. I can train 5 days/week.".to_owned())
    );
    assert_eq!(
        agent.example_outputs,
        Some(
            "Provide a week-by-week breakdown with daily workouts, paces, and recovery notes."
                .to_owned()
        )
    );
    assert_eq!(
        agent.success_criteria,
        Some("The plan should be progressive, include tapering, and respect rest days.".to_owned())
    );

    // When `instructions` is provided, system_prompt should be derived from instructions
    assert_eq!(
        agent.system_prompt,
        "You are an expert marathon coach. Build a plan based on the athlete's current fitness level, goal time, and available training days."
    );

    // Verify round-trip through get
    let fetched = manager
        .get_by_id(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.purpose, agent.purpose);
    assert_eq!(fetched.when_to_use, agent.when_to_use);
    assert_eq!(fetched.instructions, agent.instructions);
    assert_eq!(fetched.example_inputs, agent.example_inputs);
    assert_eq!(fetched.example_outputs, agent.example_outputs);
    assert_eq!(fetched.success_criteria, agent.success_criteria);
    assert_eq!(fetched.system_prompt, agent.system_prompt);
}

#[tokio::test]
async fn test_create_agent_without_structured_fields() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Flat Coach".to_owned(),
        description: Some("A coach with only system_prompt".to_owned()),
        system_prompt: "You are a helpful running coach.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Structured fields should be None
    assert!(agent.purpose.is_none());
    assert!(agent.when_to_use.is_none());
    assert!(agent.instructions.is_none());
    assert!(agent.example_inputs.is_none());
    assert!(agent.example_outputs.is_none());
    assert!(agent.success_criteria.is_none());

    // system_prompt should be the original value (no instructions to override)
    assert_eq!(agent.system_prompt, "You are a helpful running coach.");

    // Verify round-trip through get
    let fetched = manager
        .get_by_id(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap()
        .unwrap();

    assert!(fetched.purpose.is_none());
    assert!(fetched.when_to_use.is_none());
    assert!(fetched.instructions.is_none());
    assert!(fetched.example_inputs.is_none());
    assert!(fetched.example_outputs.is_none());
    assert!(fetched.success_criteria.is_none());
}

#[tokio::test]
async fn test_update_agent_adds_structured_fields() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a flat agent (no structured fields)
    let request = CreateAgentRequest {
        title: "Soon Structured".to_owned(),
        description: None,
        system_prompt: "Basic prompt".to_owned(),
        category: AgentCategory::Custom,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Verify structured fields start as None
    assert!(agent.purpose.is_none());
    assert!(agent.instructions.is_none());

    // Update to add structured fields
    let update = UpdateAgentRequest {
        title: None,
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: Some("Analyze weekly training volume.".to_owned()),
        when_to_use: Some("After completing a training week.".to_owned()),
        instructions: Some(
            "Review the athlete's weekly mileage and intensity distribution.".to_owned(),
        ),
        example_inputs: Some("Show me my training summary for this week.".to_owned()),
        example_outputs: Some("A breakdown by zone with weekly totals.".to_owned()),
        success_criteria: Some("Identify overtraining risks and recovery needs.".to_owned()),
        max_tool_iterations: FieldUpdate::Keep,
    };

    let updated = manager
        .update(
            &agent.id.to_string(),
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap()
        .unwrap();

    // Verify structured fields are now populated
    assert_eq!(
        updated.purpose,
        Some("Analyze weekly training volume.".to_owned())
    );
    assert_eq!(
        updated.when_to_use,
        Some("After completing a training week.".to_owned())
    );
    assert_eq!(
        updated.instructions,
        Some("Review the athlete's weekly mileage and intensity distribution.".to_owned())
    );
    assert_eq!(
        updated.example_inputs,
        Some("Show me my training summary for this week.".to_owned())
    );
    assert_eq!(
        updated.example_outputs,
        Some("A breakdown by zone with weekly totals.".to_owned())
    );
    assert_eq!(
        updated.success_criteria,
        Some("Identify overtraining risks and recovery needs.".to_owned())
    );

    // Verify persistence through a fresh get
    let fetched = manager
        .get_by_id(&agent.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.purpose, updated.purpose);
    assert_eq!(fetched.when_to_use, updated.when_to_use);
    assert_eq!(fetched.instructions, updated.instructions);
    assert_eq!(fetched.example_inputs, updated.example_inputs);
    assert_eq!(fetched.example_outputs, updated.example_outputs);
    assert_eq!(fetched.success_criteria, updated.success_criteria);
}

/// Writes the structured sections the admin API leaves NULL. Plain text
/// columns and `$n` placeholders, which both dialects accept.
const SET_STRUCTURED_FIELDS: &str = r"
    UPDATE agents SET
        purpose = $1, when_to_use = $2, instructions = $3,
        example_inputs = $4, example_outputs = $5, success_criteria = $6
    WHERE id = $7
    ";

#[tokio::test]
async fn test_fork_preserves_structured_fields() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    // Create a system agent via the admin API
    let system_request = CreateSystemAgentRequest {
        title: "System Structured Coach".to_owned(),
        description: Some("A system agent with structured sections".to_owned()),
        system_prompt: "You analyze race performance.".to_owned(),
        category: AgentCategory::Training,
        tags: vec!["race".to_owned(), "analysis".to_owned()],
        sample_prompts: vec![],
        visibility: AgentVisibility::Tenant,
    };

    let system_coach = manager
        .create_system_agent(test_user_id(), test_tenant(), &system_request)
        .await
        .unwrap();

    // The admin API does not insert structured fields, so set them via raw
    // SQL on whichever backend the factory opened.
    let structured_values = [
        "Analyze post-race data to identify strengths and weaknesses.",
        "After completing a race or time trial.",
        "Compare splits, heart rate zones, and pacing strategy against the goal.",
        "Here is my 10K race from last weekend.",
        "Provide split analysis, HR zone time, and pacing recommendations.",
        "Identify at least two actionable improvements for the next race.",
    ];
    let agent_id = system_coach.id.to_string();
    match &db {
        Database::SQLite(sqlite) => {
            let mut query = sqlx::query(SET_STRUCTURED_FIELDS);
            for value in structured_values {
                query = query.bind(value);
            }
            query.bind(&agent_id).execute(sqlite.pool()).await.unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => {
            let mut query = sqlx::query(SET_STRUCTURED_FIELDS);
            for value in structured_values {
                query = query.bind(value);
            }
            query
                .bind(&agent_id)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }

    // Fork the system agent as a different user in a different tenant
    let forked = manager
        .fork_agent(
            &system_coach.id.to_string(),
            other_user_id(),
            other_tenant(),
        )
        .await
        .unwrap();

    // Verify the fork preserved all structured fields
    assert_eq!(
        forked.purpose,
        Some("Analyze post-race data to identify strengths and weaknesses.".to_owned())
    );
    assert_eq!(
        forked.when_to_use,
        Some("After completing a race or time trial.".to_owned())
    );
    assert_eq!(
        forked.instructions,
        Some("Compare splits, heart rate zones, and pacing strategy against the goal.".to_owned())
    );
    assert_eq!(
        forked.example_inputs,
        Some("Here is my 10K race from last weekend.".to_owned())
    );
    assert_eq!(
        forked.example_outputs,
        Some("Provide split analysis, HR zone time, and pacing recommendations.".to_owned())
    );
    assert_eq!(
        forked.success_criteria,
        Some("Identify at least two actionable improvements for the next race.".to_owned())
    );

    // Fork should not be a system agent
    assert!(!forked.is_system);
    assert_eq!(forked.forked_from, Some(system_coach.id));

    // Verify persistence through a fresh get
    let fetched = manager
        .get_by_id(&forked.id.to_string(), other_user_id(), other_tenant())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(fetched.purpose, forked.purpose);
    assert_eq!(fetched.instructions, forked.instructions);
    assert_eq!(fetched.example_inputs, forked.example_inputs);
    assert_eq!(fetched.example_outputs, forked.example_outputs);
    assert_eq!(fetched.success_criteria, forked.success_criteria);
}

#[tokio::test]
async fn test_token_count_with_structured_fields() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let purpose = "Help athletes build a training plan.";
    let instructions = "You are an expert coach who builds periodized plans.";
    let example_inputs = "I want to run a sub-3:30 marathon.";
    let example_outputs = "Week-by-week breakdown with paces.";
    let success_criteria = "Progressive load with proper taper.";

    let request = CreateAgentRequest {
        title: "Token Count Coach".to_owned(),
        description: None,
        system_prompt: "This fallback prompt should not be used for token counting.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: Some(purpose.to_owned()),
        when_to_use: Some("When preparing for a race.".to_owned()),
        instructions: Some(instructions.to_owned()),
        example_inputs: Some(example_inputs.to_owned()),
        example_outputs: Some(example_outputs.to_owned()),
        success_criteria: Some(success_criteria.to_owned()),
        max_tool_iterations: None,
    };

    let agent = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    // Token count should be computed from sections (purpose + instructions +
    // example_inputs + example_outputs + success_criteria) / 4.
    // when_to_use is NOT counted.
    let total_chars = purpose.len()
        + instructions.len()
        + example_inputs.len()
        + example_outputs.len()
        + success_criteria.len();
    #[allow(clippy::cast_possible_truncation)]
    let expected_token_count = (total_chars / 4) as u32;

    assert_eq!(agent.token_count, expected_token_count);

    // Also verify it does NOT equal system_prompt-based count
    #[allow(clippy::cast_possible_truncation)]
    let system_prompt_token_count =
        ("This fallback prompt should not be used for token counting.".len() / 4) as u32;
    assert_ne!(
        agent.token_count, system_prompt_token_count,
        "Token count should come from structured sections, not system_prompt"
    );
}

/// `max_tool_iterations` is the per-agent tool-call budget the chat pipeline
/// reads to override the admin default. `AgentsRepository::create` must persist a
/// caller-supplied value, not just carry it in the returned struct — so this
/// asserts the concrete budget on a fresh read back out of the database.
#[tokio::test]
async fn test_create_agent_persists_max_tool_iterations() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Budgeted Coach".to_owned(),
        description: None,
        system_prompt: "You are a coach with a wide tool budget.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: Some(23),
    };

    let created = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    assert_eq!(created.max_tool_iterations, Some(23));

    // Re-read: proves the value reached the agents row, not just the struct
    // the create path returns.
    let fetched = manager
        .get_by_id(&created.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.max_tool_iterations, Some(23));
}

/// `AgentsRepository::update` must write a new `max_tool_iterations` over the
/// stored one. The update path resolves the request against the stored value,
/// so an agent created with a budget and updated to a different budget proves
/// the request value wins rather than the existing one leaking through.
#[tokio::test]
async fn test_update_agent_writes_max_tool_iterations() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Budgeted Coach".to_owned(),
        description: None,
        system_prompt: "You are a coach with a wide tool budget.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: Some(23),
    };

    let created = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let update = UpdateAgentRequest {
        title: None,
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Set(Some(7)),
    };

    let updated = manager
        .update(
            &created.id.to_string(),
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.max_tool_iterations, Some(7));

    let fetched = manager
        .get_by_id(&created.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.max_tool_iterations, Some(7));
}

/// An update that carries no budget must leave the stored one alone. Without
/// [`FieldUpdate::Keep`] resolving to the existing value, an unrelated edit (a
/// title change) would silently reset the agent's tool budget to the admin
/// default.
#[tokio::test]
async fn test_update_agent_preserves_max_tool_iterations_when_absent() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Budgeted Coach".to_owned(),
        description: None,
        system_prompt: "You are a coach with a wide tool budget.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: Some(23),
    };

    let created = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();

    let update = UpdateAgentRequest {
        title: Some("Renamed Coach".to_owned()),
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Keep,
    };

    let updated = manager
        .update(
            &created.id.to_string(),
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.title, "Renamed Coach");
    assert_eq!(updated.max_tool_iterations, Some(23));
}

/// A budget you can set but never unset is unfinished. `FieldUpdate::Set(None)`
/// is the wire's explicit `null` — the update must write NULL over the stored
/// 23 so the agent goes back to inheriting the admin value, which the old
/// `request.or(existing)` coalesce could never do.
#[tokio::test]
async fn test_update_agent_clears_max_tool_iterations_on_an_explicit_null() {
    let db = create_test_db().await;
    let manager = db.repositories().agents;

    let request = CreateAgentRequest {
        title: "Budgeted Coach".to_owned(),
        description: None,
        system_prompt: "You are a coach with a wide tool budget.".to_owned(),
        category: AgentCategory::Training,
        tags: vec![],
        sample_prompts: vec![],
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: Some(23),
    };

    let created = manager
        .create(test_user_id(), test_tenant(), &request)
        .await
        .unwrap();
    assert_eq!(created.max_tool_iterations, Some(23));

    let update = UpdateAgentRequest {
        title: None,
        description: None,
        system_prompt: None,
        category: None,
        tags: None,
        sample_prompts: None,
        startup_query: None,
        data_requirements: None,
        purpose: None,
        when_to_use: None,
        instructions: None,
        example_inputs: None,
        example_outputs: None,
        success_criteria: None,
        max_tool_iterations: FieldUpdate::Set(None),
    };

    let updated = manager
        .update(
            &created.id.to_string(),
            test_user_id(),
            test_tenant(),
            &update,
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.max_tool_iterations, None);
    // Title untouched: clearing the budget is not a wholesale reset.
    assert_eq!(updated.title, "Budgeted Coach");

    // Re-read: proves NULL reached the agents row, not just the struct the
    // update path returns.
    let fetched = manager
        .get_by_id(&created.id.to_string(), test_user_id(), test_tenant())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.max_tool_iterations, None);
}
