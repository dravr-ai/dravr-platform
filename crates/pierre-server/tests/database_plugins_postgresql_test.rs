// ABOUTME: PostgreSQL-specific tests for Tool Selection and Chat database methods
// ABOUTME: Tests PostgreSQL implementation of DatabaseProvider trait for new features
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![cfg(feature = "postgresql")]

use chrono::Utc;
use pierre_database::{backends::factory::Database, database::AddMessageParams};
use pierre_mcp_server::{
    models::{Tenant, TenantId, TenantPlan, ToolCategory, User, UserStatus, UserTier},
    permissions::UserRole,
};
use uuid::Uuid;

mod common;

// ============================================================================
// PostgreSQL Tool Selection Tests
// ============================================================================

#[tokio::test]
async fn test_pg_get_tool_catalog() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let catalog = repos
        .tool_selection
        .get_tool_catalog()
        .await
        .expect("Failed to get catalog");

    // Catalog should be populated from migrations
    assert!(!catalog.is_empty(), "Tool catalog should not be empty");

    // Check that entries have required fields
    for entry in &catalog {
        assert!(!entry.id.is_empty(), "Tool ID should not be empty");
        assert!(!entry.tool_name.is_empty(), "Tool name should not be empty");
        assert!(
            !entry.display_name.is_empty(),
            "Display name should not be empty"
        );
    }
}

#[tokio::test]
async fn test_pg_get_tool_catalog_entry() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    // Test getting an existing tool
    let entry = repos
        .tool_selection
        .get_tool_catalog_entry("get_activities")
        .await
        .expect("Failed to get tool entry");

    assert!(entry.is_some(), "get_activities should exist in catalog");
    let entry = entry.unwrap();
    assert_eq!(entry.tool_name, "get_activities");
    assert_eq!(entry.category, ToolCategory::Fitness);

    // Test getting a non-existent tool
    let missing = repos
        .tool_selection
        .get_tool_catalog_entry("nonexistent_tool")
        .await
        .expect("Query should not fail");
    assert!(missing.is_none(), "Non-existent tool should return None");
}

#[tokio::test]
async fn test_pg_get_tools_by_category() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let fitness_tools = repos
        .tool_selection
        .get_tools_by_category(ToolCategory::Fitness)
        .await
        .expect("Failed to get fitness tools");

    assert!(
        !fitness_tools.is_empty(),
        "Should have fitness category tools"
    );

    // All returned tools should be in the fitness category
    for tool in &fitness_tools {
        assert_eq!(
            tool.category,
            ToolCategory::Fitness,
            "Tool {} should be in Fitness category",
            tool.tool_name
        );
    }
}

#[tokio::test]
async fn test_pg_get_tools_by_min_plan() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    // Starter plan should get starter tools
    let starter_tools = repos
        .tool_selection
        .get_tools_by_min_plan(TenantPlan::Starter)
        .await
        .expect("Failed to get starter tools");

    assert!(!starter_tools.is_empty(), "Should have starter tools");

    // Enterprise plan should get all tools
    let enterprise_tools = repos
        .tool_selection
        .get_tools_by_min_plan(TenantPlan::Enterprise)
        .await
        .expect("Failed to get enterprise tools");

    assert!(
        enterprise_tools.len() >= starter_tools.len(),
        "Enterprise should have at least as many tools as Starter"
    );
}

#[tokio::test]
async fn test_pg_tenant_tool_overrides() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    // Create a test tenant
    let tenant_id = TenantId::new();
    let user_id = create_pg_test_user(&db).await;
    create_pg_test_tenant(&db, tenant_id, user_id).await;

    // Initially no overrides
    let overrides = repos
        .tool_selection
        .get_overrides(tenant_id)
        .await
        .expect("Failed to get overrides");
    assert!(overrides.is_empty(), "New tenant should have no overrides");

    // Create an override
    let created = repos
        .tool_selection
        .upsert_override(
            tenant_id,
            "get_activities",
            false,
            Some(user_id),
            Some("Disabled for testing".to_owned()),
        )
        .await
        .expect("Failed to create override");

    assert_eq!(created.tool_name, "get_activities");
    assert!(!created.is_enabled);

    // Get the single override
    let single = repos
        .tool_selection
        .get_override(tenant_id, "get_activities")
        .await
        .expect("Failed to get single override");
    assert!(single.is_some(), "Should find the override");

    // Update the override
    let updated = repos
        .tool_selection
        .upsert_override(tenant_id, "get_activities", true, Some(user_id), None)
        .await
        .expect("Failed to update override");
    assert!(updated.is_enabled, "Override should now be enabled");

    // Delete the override
    let deleted = repos
        .tool_selection
        .delete_override(tenant_id, "get_activities")
        .await
        .expect("Failed to delete override");
    assert!(deleted, "Delete should return true");

    // Verify deletion
    let after_delete = repos
        .tool_selection
        .get_override(tenant_id, "get_activities")
        .await
        .expect("Query should not fail");
    assert!(after_delete.is_none(), "Override should be deleted");
}

#[tokio::test]
async fn test_pg_count_enabled_tools() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    // Create a test tenant with starter plan
    let tenant_id = TenantId::new();
    let user_id = create_pg_test_user(&db).await;
    create_pg_test_tenant(&db, tenant_id, user_id).await;

    let count = repos
        .tool_selection
        .count_enabled_tools(tenant_id)
        .await
        .expect("Failed to count enabled tools");

    assert!(count > 0, "Should have some enabled tools");
}

// ============================================================================
// PostgreSQL Chat Tests
// ============================================================================

#[tokio::test]
async fn test_pg_chat_create_conversation() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    let conv = repos
        .chat
        .create_conversation(&user_id_str, tenant_id, "Test Chat", "gpt-4", None)
        .await
        .expect("Failed to create conversation");

    assert!(!conv.id.is_empty(), "Conversation ID should be set");
    assert_eq!(conv.title, "Test Chat");
    assert_eq!(conv.model, "gpt-4");
    assert!(conv.coach_id.is_none());
}

#[tokio::test]
async fn test_pg_chat_create_conversation_without_coach_defaults_to_none() {
    // With coach_id reified as an FK, plain conversations default to None and
    // the runtime resolves the default Pierre prompt. Full coach-attached
    // flows are exercised in the orchestration integration tests which seed
    // a coaches row first.
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    let conv = repos
        .chat
        .create_conversation(&user_id_str, tenant_id, "Test with Prompt", "gpt-4", None)
        .await
        .expect("Failed to create conversation");

    assert!(conv.coach_id.is_none());
    assert_eq!(conv.title, "Test with Prompt");
}

#[tokio::test]
async fn test_pg_chat_get_conversation() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    // Create a conversation first
    let created = repos
        .chat
        .create_conversation(&user_id_str, tenant_id, "Retrieve Test", "gpt-4", None)
        .await
        .expect("Failed to create conversation");

    // Retrieve it
    let retrieved = repos
        .chat
        .get_conversation(&created.id, &user_id_str, tenant_id)
        .await
        .expect("Failed to get conversation");

    assert!(retrieved.is_some(), "Should find the conversation");
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, created.id);
    assert_eq!(retrieved.title, "Retrieve Test");

    // Try to get non-existent
    let missing = repos
        .chat
        .get_conversation("nonexistent", &user_id_str, tenant_id)
        .await
        .expect("Query should not fail");
    assert!(
        missing.is_none(),
        "Non-existent conversation should be None"
    );
}

#[tokio::test]
async fn test_pg_chat_list_conversations() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    // Create multiple conversations
    for i in 1..=5 {
        repos
            .chat
            .create_conversation(&user_id_str, tenant_id, &format!("Chat {i}"), "gpt-4", None)
            .await
            .expect("Failed to create conversation");
    }

    // List with pagination
    let list = repos
        .chat
        .list_conversations(&user_id_str, tenant_id, 3, 0)
        .await
        .expect("Failed to list conversations");

    assert_eq!(list.len(), 3, "Should return 3 conversations with limit");

    // List second page
    let page2 = repos
        .chat
        .list_conversations(&user_id_str, tenant_id, 3, 3)
        .await
        .expect("Failed to list page 2");

    assert_eq!(page2.len(), 2, "Should return remaining 2 conversations");
}

#[tokio::test]
async fn test_pg_chat_update_conversation_title() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    let conv = repos
        .chat
        .create_conversation(&user_id_str, tenant_id, "Original Title", "gpt-4", None)
        .await
        .expect("Failed to create conversation");

    let updated = repos
        .chat
        .update_conversation_title(&conv.id, &user_id_str, tenant_id, "Updated Title")
        .await
        .expect("Failed to update title");

    assert!(updated, "Update should succeed");

    let retrieved = repos
        .chat
        .get_conversation(&conv.id, &user_id_str, tenant_id)
        .await
        .expect("Failed to get conversation")
        .unwrap();

    assert_eq!(retrieved.title, "Updated Title");
}

#[tokio::test]
async fn test_pg_chat_delete_conversation() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    let conv = repos
        .chat
        .create_conversation(&user_id_str, tenant_id, "To Delete", "gpt-4", None)
        .await
        .expect("Failed to create conversation");

    let deleted = repos
        .chat
        .delete_conversation(&conv.id, &user_id_str, tenant_id)
        .await
        .expect("Failed to delete conversation");

    assert!(deleted, "Delete should succeed");

    let retrieved = repos
        .chat
        .get_conversation(&conv.id, &user_id_str, tenant_id)
        .await
        .expect("Query should not fail");

    assert!(retrieved.is_none(), "Deleted conversation should not exist");
}

#[tokio::test]
async fn test_pg_chat_messages() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    let conv = repos
        .chat
        .create_conversation(&user_id_str, tenant_id, "Message Test", "gpt-4", None)
        .await
        .expect("Failed to create conversation");

    // Add messages (user_id required for ownership verification)
    let msg1_params = AddMessageParams {
        conversation_id: &conv.id,
        user_id: &user_id_str,
        role: "user",
        content: "Hello!",
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
    };
    let msg1 = repos
        .chat
        .add_message(&msg1_params)
        .await
        .expect("Failed to add user message");

    assert_eq!(msg1.role, "user");
    assert_eq!(msg1.content, "Hello!");

    let msg2_params = AddMessageParams {
        conversation_id: &conv.id,
        user_id: &user_id_str,
        role: "assistant",
        content: "Hi there!",
        token_count: Some(10),
        finish_reason: Some("stop"),
        prompt_tokens: None,
        model: None,
    };
    let msg2 = repos
        .chat
        .add_message(&msg2_params)
        .await
        .expect("Failed to add assistant message");

    assert_eq!(msg2.role, "assistant");
    assert_eq!(msg2.token_count, Some(10));
    assert_eq!(msg2.finish_reason, Some("stop".to_owned()));

    // Get all messages (user_id required for ownership verification)
    let messages = repos
        .chat
        .get_messages(&conv.id, &user_id_str)
        .await
        .expect("Failed to get messages");

    assert_eq!(messages.len(), 2, "Should have 2 messages");

    // Get recent messages (user_id required for ownership verification)
    let recent = repos
        .chat
        .get_recent_messages(&conv.id, &user_id_str, 1)
        .await
        .expect("Failed to get recent messages");

    assert_eq!(recent.len(), 1, "Should return only 1 recent message");

    // Get message count (user_id required for ownership verification)
    let count = repos
        .chat
        .get_message_count(&conv.id, &user_id_str)
        .await
        .expect("Failed to get message count");

    assert_eq!(count, 2, "Should have 2 messages");
}

#[tokio::test]
async fn test_pg_chat_delete_all_user_conversations() {
    let isolated_db = match common::IsolatedPostgresDb::new().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test: PostgreSQL not available: {e}");
            return;
        }
    };

    let db = isolated_db
        .get_database()
        .await
        .expect("Failed to get database");
    let repos = db.repositories();

    let user_id = create_pg_test_user(&db).await;
    let user_id_str = user_id.to_string();
    let tenant_id = TenantId::from(Uuid::new_v4());

    // Create multiple conversations
    for i in 1..=3 {
        repos
            .chat
            .create_conversation(&user_id_str, tenant_id, &format!("Conv {i}"), "gpt-4", None)
            .await
            .expect("Failed to create conversation");
    }

    // Delete all
    let deleted_count = repos
        .chat
        .delete_all_user_conversations(&user_id_str, tenant_id)
        .await
        .expect("Failed to delete all conversations");

    assert_eq!(deleted_count, 3, "Should delete 3 conversations");

    // Verify
    let remaining = repos
        .chat
        .list_conversations(&user_id_str, tenant_id, 100, 0)
        .await
        .expect("Failed to list conversations");

    assert!(remaining.is_empty(), "No conversations should remain");
}

// ============================================================================
// Helper Functions
// ============================================================================

async fn create_pg_test_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("test-{user_id}@example.com"),
        display_name: Some("Test User".to_owned()),
        password_hash: "test_hash".to_owned(),
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
        default_coach_id: None,
    };

    db.repositories()
        .users
        .create(&user)
        .await
        .expect("Failed to create user");
    user_id
}

async fn create_pg_test_tenant(db: &Database, tenant_id: TenantId, owner_id: Uuid) {
    let tenant = Tenant {
        id: tenant_id,
        name: "Test Tenant".to_owned(),
        slug: format!("test-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: owner_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    db.repositories()
        .tenants
        .create(&tenant)
        .await
        .expect("Failed to create tenant");
}
