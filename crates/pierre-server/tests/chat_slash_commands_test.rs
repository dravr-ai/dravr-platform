// ABOUTME: Integration tests for slash-command handling on the /api/chat/.../messages endpoint
// ABOUTME: Asserts web and mobile chat use the same dispatcher as the messaging channels
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use axum::http::StatusCode;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::models::{Tenant, User, UserStatus};
use pierre_mcp_server::routes::chat::{ChatCompletionResponse, ChatRoutes, ConversationResponse};
use serde_json::json;
use std::sync::Arc;
use tokio::task::spawn_blocking;
use uuid::Uuid;

async fn seed_user_tenant(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Chat Cmd Test".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: "Chat Cmd Tenant".to_owned(),
        slug: format!("chat-cmd-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    // tenants.create() adds the owner to tenant_users automatically.
    resources.repos.tenants.create(&tenant).await.unwrap();
    // Legacy tenant_id column mirrors tenant_users; keep them in sync
    // for the /api/chat endpoints that still read it.
    resources
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let token = resources
        .auth_manager
        .generate_token(&user, &resources.jwks_manager)
        .unwrap();

    (user_id, tenant_id, format!("Bearer {token}"))
}

async fn seed_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    description: &str,
) -> String {
    let request = CreateCoachRequest {
        title: title.to_owned(),
        description: Some(description.to_owned()),
        system_prompt: "You are a test coach.".to_owned(),
        category: CoachCategory::Training,
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
    };
    let coach = resources
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    coach.id.to_string()
}

async fn create_conversation(router: axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", auth)
        .json(&json!({
            "title": "Cmd Test",
            "model": "gemini-1.5-flash"
        }))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = resp.json();
    conv.id
}

#[tokio::test]
async fn coach_command_returns_card_with_actions_no_llm_call() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "web-chat@test.com").await;
    let _coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Activity Analysis Coach",
        "Analyzes training data. What *not* to overlook.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/coach"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: ChatCompletionResponse = resp.json();

    // Command response marker is set so clients can skip LLM-specific UI.
    assert!(body.is_command_response);
    // Card title present for /coach (list card).
    assert!(body.card_title.is_some(), "expected card_title on /coach");
    // Actions populated with per-coach select buttons.
    let actions = body.actions.expect("actions array present");
    assert!(!actions.is_empty(), "expected at least one coach action");
    assert_eq!(actions[0].action_type, "postback");
    assert!(
        actions[0].value.starts_with("/coach select "),
        "action value should be a postback to /coach select, got: {}",
        actions[0].value
    );
    // Markdown emphasis stripped uniformly (same behaviour as messaging channels).
    assert!(
        !body.assistant_message.content.contains('*'),
        "command response must not contain literal markdown asterisks: {}",
        body.assistant_message.content
    );
    // No LLM tokens billed for a command turn.
    assert_eq!(body.execution_time_ms, 0);
    assert_eq!(body.model, "command");
}

#[tokio::test]
async fn coach_select_in_chat_sets_users_default_coach() {
    // Mirrors the Telegram-DM regression test: calling /coach select
    // from the web chat endpoint must write users.default_coach_id, the
    // same behaviour the messaging dispatcher gives in a DM.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "coach-sel@test.com").await;
    let coach_id = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Polarized Training Coach",
        "Experts in training intensity distribution.",
    )
    .await;

    let before = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .unwrap();
    assert!(before.default_coach_id.is_none());

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": format!("/coach select {coach_id}")}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: ChatCompletionResponse = resp.json();
    assert!(body.is_command_response);
    // DM wording (KEY_COACH_USER_UPDATED) — never "pour le groupe".
    assert!(
        !body
            .assistant_message
            .content
            .to_lowercase()
            .contains("group"),
        "web/mobile chat is DM-like; must not mention 'group': {}",
        body.assistant_message.content
    );

    let after = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.default_coach_id.as_deref(), Some(coach_id.as_str()));
}

#[tokio::test]
async fn unknown_slash_command_short_circuits_before_llm() {
    let resources = create_test_server_resources().await.unwrap();
    let (_uid, _tid, auth) = seed_user_tenant(&resources, "unknown-cmd@test.com").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .json(&json!({"content": "/notarealcommand"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: ChatCompletionResponse = resp.json();
    assert!(body.is_command_response);
    // No card, no actions — unknown commands render as plain localized text.
    assert!(body.card_title.is_none());
    assert!(body.actions.is_none());
    assert_eq!(body.model, "command");
}

#[tokio::test]
async fn client_platform_header_shapes_channel_type_without_breaking() {
    // Web and mobile currently share the same dispatch semantics, but
    // the X-Client-Platform header still rides through the pipeline for
    // analytics. Smoke-test that mobile flag is accepted (no 400).
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "mobile-cmd@test.com").await;
    let _coach = seed_coach(
        &resources,
        user_id,
        tenant_id,
        "Recovery Coach",
        "Rest-day specialist.",
    )
    .await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv_id = create_conversation(router.clone(), &auth).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", &auth)
        .header("x-client-platform", "mobile")
        .json(&json!({"content": "/coach"}))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: ChatCompletionResponse = resp.json();
    assert!(body.is_command_response);
}
