// ABOUTME: Integration tests for admin analytics endpoints
// ABOUTME: Tests the /api/admin/analytics/recent-activity endpoint with seeded data
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

mod common;
mod helpers;

use anyhow::Result;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{AddMessageParams, ConversationTurnId, InsertLlmUsage, TenantId};
use pierre_mcp_server::{
    mcp::resources::ServerResources,
    models::{Tenant, User, UserStatus},
    permissions::UserRole,
    routes::WebAdminRoutes,
};
use serde_json::Value;
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

/// Build a `WebAdminRoutes` router backed by the given server resources
fn build_admin_analytics_router(resources: Arc<ServerResources>) -> axum::Router {
    WebAdminRoutes::routes(resources)
}

/// Create an admin user from scratch with `is_admin` set before insertion
async fn create_admin_user_and_token(
    resources: &Arc<ServerResources>,
    email: &str,
) -> (Uuid, String) {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Admin User".to_owned()),
    );
    user.is_admin = true;
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.repos.users.create(&user).await.unwrap();

    // Create tenant for the admin user
    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Admin Tenant for {email}"),
        slug: format!("admin-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources.repos.tenants.create(&tenant).await.unwrap();
    resources
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    (user_id, format!("Bearer {token}"))
}

/// Create a regular (non-admin) user from scratch, returning the Bearer token
async fn create_regular_user_and_token(resources: &Arc<ServerResources>, email: &str) -> String {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Regular User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.repos.users.create(&user).await.unwrap();

    // Create tenant for the regular user
    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant for {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources.repos.tenants.create(&tenant).await.unwrap();
    resources
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    format!("Bearer {token}")
}

/// Seed LLM usage records into the database and return the count inserted
async fn seed_llm_usage(
    resources: &Arc<ServerResources>,
    tenant_id: &str,
    user_id: &str,
    count: usize,
) -> usize {
    let providers = ["google", "openai", "groq"];
    let models = ["gemini-2.0-flash", "gpt-4o-mini", "llama-3.3-70b"];

    for i in 0..count {
        let idx = i % providers.len();
        let offset = i as i64;
        let params = InsertLlmUsage {
            tenant_id,
            user_id,
            conversation_id: None,
            turn_id: ConversationTurnId::new(),
            provider: providers[idx],
            model: models[idx],
            prompt_tokens: 100 + (offset * 10),
            completion_tokens: 50 + (offset * 5),
            total_tokens: 150 + (offset * 15),
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(200 + (offset * 50)),
        };
        resources
            .repos
            .llm_usage
            .insert_llm_usage(&params)
            .await
            .unwrap();
    }

    count
}

/// Seed chat conversations and messages, returning counts of each
async fn seed_conversations_and_messages(
    resources: &Arc<ServerResources>,
    user_id: &str,
    tenant_id: TenantId,
    conversation_count: usize,
    messages_per_conversation: usize,
) -> (usize, usize) {
    let mut total_messages = 0;

    for i in 0..conversation_count {
        let title = format!("Test Conversation {i}");
        let convo = resources
            .repos
            .chat
            .create_conversation(user_id, tenant_id, &title, "gemini-2.0-flash", None)
            .await
            .unwrap();

        for j in 0..messages_per_conversation {
            let role = if j % 2 == 0 { "user" } else { "assistant" };
            let content = format!("Message {j} in conversation {i}");
            let params = AddMessageParams {
                conversation_id: &convo.id,
                user_id,
                role,
                content: &content,
                token_count: Some(50),
                finish_reason: Some("stop"),
                prompt_tokens: Some(30),
                model: Some("gemini-2.0-flash"),
            };
            resources.repos.chat.add_message(&params).await.unwrap();
            total_messages += 1;
        }
    }

    (conversation_count, total_messages)
}

// ============================================================================
// Test 1: Recent activity endpoint returns LLM calls and conversations
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recent_activity_returns_seeded_data() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (user_id, auth_token) =
        create_admin_user_and_token(&resources, "analytics_admin@test.com").await;

    // Look up tenant for seeding data
    let tenants = resources.repos.tenants.list_for_user(user_id).await?;
    let tenant = tenants.first().expect("admin user should have a tenant");
    let tenant_id = tenant.id;
    let user_id_str = user_id.to_string();
    let tenant_id_str = tenant_id.to_string();

    // Seed 5 LLM usage records
    let llm_count = seed_llm_usage(&resources, &tenant_id_str, &user_id_str, 5).await;
    assert_eq!(llm_count, 5);

    // Seed 3 conversations with 4 messages each
    let (conv_count, msg_count) =
        seed_conversations_and_messages(&resources, &user_id_str, tenant_id, 3, 4).await;
    assert_eq!(conv_count, 3);
    assert_eq!(msg_count, 12);

    let router = build_admin_analytics_router(resources);

    let response = AxumTestRequest::get("/api/admin/analytics/recent-activity")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status(), 200);

    let body: Value = response.json();

    // Verify LLM calls are returned
    let recent_llm = body["recent_llm_calls"]
        .as_array()
        .expect("recent_llm_calls should be an array");
    assert!(
        recent_llm.len() >= 5,
        "Expected at least 5 LLM calls, got {}",
        recent_llm.len()
    );

    // Verify each LLM call has required fields
    for call in recent_llm {
        assert!(
            call["provider"].is_string(),
            "LLM call should have provider"
        );
        assert!(call["model"].is_string(), "LLM call should have model");
        assert!(
            call["total_tokens"].is_number(),
            "LLM call should have total_tokens"
        );
        assert!(
            call["cost_usd"].is_number(),
            "LLM call should have cost_usd"
        );
        assert!(
            call["call_type"].is_string(),
            "LLM call should have call_type"
        );
    }

    // Verify conversations are returned
    let recent_convos = body["recent_conversations"]
        .as_array()
        .expect("recent_conversations should be an array");
    assert!(
        recent_convos.len() >= 3,
        "Expected at least 3 conversations, got {}",
        recent_convos.len()
    );

    // Verify each conversation has required fields
    for convo in recent_convos {
        assert!(convo["id"].is_string(), "Conversation should have id");
        assert!(convo["title"].is_string(), "Conversation should have title");
        assert!(
            convo["updated_at"].is_string(),
            "Conversation should have updated_at"
        );
    }

    // Verify summary statistics
    let summary = &body["summary"];
    assert!(
        summary["llm_calls_today"].as_i64().unwrap_or(0) >= 5,
        "Expected llm_calls_today >= 5, got {:?}",
        summary["llm_calls_today"]
    );
    assert!(
        summary["total_tokens_today"].as_i64().unwrap_or(0) > 0,
        "Expected total_tokens_today > 0"
    );

    Ok(())
}

// ============================================================================
// Test 2: Recent activity with no data returns empty arrays and zero counts
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recent_activity_empty_database() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_user_id, auth_token) =
        create_admin_user_and_token(&resources, "empty_admin@test.com").await;

    let router = build_admin_analytics_router(resources);

    let response = AxumTestRequest::get("/api/admin/analytics/recent-activity")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status(), 200);

    let body: Value = response.json();

    // Empty arrays for recent items
    let llm_calls = body["recent_llm_calls"]
        .as_array()
        .expect("recent_llm_calls should be an array");
    assert!(
        llm_calls.is_empty(),
        "Expected empty LLM calls on fresh database"
    );

    let convos = body["recent_conversations"]
        .as_array()
        .expect("recent_conversations should be an array");
    assert!(
        convos.is_empty(),
        "Expected empty conversations on fresh database"
    );

    // Summary should have zero counts
    let summary = &body["summary"];
    assert_eq!(summary["llm_calls_today"].as_i64().unwrap_or(-1), 0);
    assert_eq!(summary["total_tokens_today"].as_i64().unwrap_or(-1), 0);
    assert_eq!(summary["active_conversations"].as_i64().unwrap_or(-1), 0);
    assert!(
        summary["estimated_cost_today"].as_f64().unwrap_or(-1.0) == 0.0,
        "Expected estimated_cost_today == 0.0 on empty database"
    );

    Ok(())
}

// ============================================================================
// Test 3: Non-admin user gets 403 Forbidden
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recent_activity_non_admin_forbidden() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let auth_token = create_regular_user_and_token(&resources, "regular_user@test.com").await;

    let router = build_admin_analytics_router(resources);

    let response = AxumTestRequest::get("/api/admin/analytics/recent-activity")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    // The admin guard returns 403 for non-admin users
    assert_eq!(
        response.status(),
        403,
        "Non-admin user should receive 403 Forbidden"
    );

    Ok(())
}

// ============================================================================
// Test 4: Unauthenticated request gets 401
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recent_activity_unauthenticated() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let router = build_admin_analytics_router(resources);

    let response = AxumTestRequest::get("/api/admin/analytics/recent-activity")
        .send(router)
        .await;

    assert_eq!(
        response.status(),
        401,
        "Request without auth should receive 401 Unauthorized"
    );

    Ok(())
}

// ============================================================================
// Test 5: Recent activity includes cost estimation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recent_activity_cost_estimation() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (user_id, auth_token) =
        create_admin_user_and_token(&resources, "cost_admin@test.com").await;

    let tenants = resources.repos.tenants.list_for_user(user_id).await?;
    let tenant = tenants.first().expect("admin user should have a tenant");
    let tenant_id_str = tenant.id.to_string();
    let user_id_str = user_id.to_string();

    // Seed LLM usage with known token counts
    seed_llm_usage(&resources, &tenant_id_str, &user_id_str, 3).await;

    let router = build_admin_analytics_router(resources);

    let response = AxumTestRequest::get("/api/admin/analytics/recent-activity")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status(), 200);

    let body: Value = response.json();

    // Verify each LLM call has a non-negative cost
    let recent_llm = body["recent_llm_calls"]
        .as_array()
        .expect("recent_llm_calls should be an array");
    for call in recent_llm {
        let cost = call["cost_usd"]
            .as_f64()
            .expect("cost_usd should be a number");
        assert!(cost >= 0.0, "Cost should be non-negative, got {cost}");
    }

    // Verify the estimated_cost_today field exists and is non-negative
    let summary = &body["summary"];
    let estimated_cost = summary["estimated_cost_today"]
        .as_f64()
        .expect("estimated_cost_today should be a number");
    assert!(
        estimated_cost >= 0.0,
        "Estimated cost should be non-negative, got {estimated_cost}"
    );

    Ok(())
}

// ============================================================================
// Test 6: Recent activity with many records respects the limit
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recent_activity_respects_limit() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (user_id, auth_token) =
        create_admin_user_and_token(&resources, "limit_admin@test.com").await;

    let tenants = resources.repos.tenants.list_for_user(user_id).await?;
    let tenant = tenants.first().expect("admin user should have a tenant");
    let tenant_id = tenant.id;
    let tenant_id_str = tenant_id.to_string();
    let user_id_str = user_id.to_string();

    // Seed 30 LLM usage records (more than the default limit of 20)
    seed_llm_usage(&resources, &tenant_id_str, &user_id_str, 30).await;

    // Seed 25 conversations (more than the default limit of 20)
    seed_conversations_and_messages(&resources, &user_id_str, tenant_id, 25, 1).await;

    let router = build_admin_analytics_router(resources);

    let response = AxumTestRequest::get("/api/admin/analytics/recent-activity")
        .header("authorization", &auth_token)
        .send(router)
        .await;

    assert_eq!(response.status(), 200);

    let body: Value = response.json();

    // The handler limits recent items to 20
    let max_recent_limit: usize = 20;

    let recent_llm = body["recent_llm_calls"]
        .as_array()
        .expect("recent_llm_calls should be an array");
    assert!(
        recent_llm.len() <= max_recent_limit,
        "LLM calls should be capped at {max_recent_limit}, got {}",
        recent_llm.len()
    );

    let recent_convos = body["recent_conversations"]
        .as_array()
        .expect("recent_conversations should be an array");
    assert!(
        recent_convos.len() <= max_recent_limit,
        "Conversations should be capped at {max_recent_limit}, got {}",
        recent_convos.len()
    );

    // But the summary should still count ALL calls today, not just the recent ones
    let summary = &body["summary"];
    assert!(
        summary["llm_calls_today"].as_i64().unwrap_or(0) >= 30,
        "Summary should count all 30 LLM calls, got {:?}",
        summary["llm_calls_today"]
    );

    Ok(())
}
