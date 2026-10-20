// ABOUTME: Integration tests for group-scoped chat conversations created over REST
// ABOUTME: Pins that group_id round-trips for a member and is refused for a non-member
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::agents::{Agent, CreateAgentRequest};
use pierre_core::models::{CreateGroupRequest, TenantId};
use pierre_groups::strategies::tier::EnterpriseTierStrategy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse};
use uuid::Uuid;

/// The chat router, over a group its owner created through `GroupService` —
/// the code `/group create` runs — with an agent persona stored for it.
struct Fixture {
    router: axum::Router,
    /// Owner of the group, in the shared tenant.
    owner_auth: String,
    /// A second real user in the same tenant who never joins the group.
    outsider_auth: String,
    group_id: String,
    res: Arc<ServerContext>,
    owner_id: Uuid,
    shared_tid: TenantId,
}

async fn setup() -> Fixture {
    let res = create_test_server_resources().await.unwrap();

    let (owner_id, owner, _owner_tid) = create_test_user_with_plan(
        &res.agent.database,
        "groupchatowner@test.com",
        "professional",
    )
    .await
    .unwrap();
    let (outsider_id, outsider, _outsider_tid) = create_test_user_with_plan(
        &res.agent.database,
        "groupchatoutsider@test.com",
        "professional",
    )
    .await
    .unwrap();

    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);

    // The outsider acts inside the owner's tenant so the refusal under test is
    // the group-membership check, not tenant isolation. That means a real
    // `tenant_users` row: a token that merely claims the tenant is rejected at
    // authentication with a 401, which would pass a status assertion for the
    // wrong reason.
    let repos = res.agent.database.repositories();
    let shared_tid = repos
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    repos
        .users
        .update_tenant_id(outsider_id, shared_tid)
        .await
        .unwrap();
    let outsider_auth = format!(
        "Bearer {}",
        res.auth
            .auth_manager
            .generate_token_with_tenant(
                &outsider,
                &res.auth.jwks_manager,
                Some(shared_tid.to_string()),
            )
            .unwrap()
    );

    let router = ChatRoutes::routes(Arc::clone(&res));

    let agent_request: CreateAgentRequest = serde_json::from_value(json!({
        "title": "Squad Coach",
        "system_prompt": "Coach the squad.",
        "category": "training",
        "tags": ["run"]
    }))
    .unwrap();
    let agent_id = repos
        .agents
        .create(owner_id, shared_tid, &agent_request)
        .await
        .unwrap()
        .id
        .to_string();

    let group_request = CreateGroupRequest {
        name: "Marathon Squad".to_owned(),
        description: None,
        agent_id,
        max_members: None,
        coach_user_id: None,
    };
    let group_id = res
        .group_service()
        .create_group(
            &group_request,
            owner_id,
            shared_tid,
            &EnterpriseTierStrategy,
        )
        .await
        .unwrap()
        .id
        .to_string();

    Fixture {
        router,
        owner_auth,
        outsider_auth,
        group_id,
        res,
        owner_id,
        shared_tid,
    }
}

#[tokio::test]
async fn test_conversation_created_with_group_id_carries_it_back() {
    let fx = Box::pin(setup()).await;

    let response = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&json!({
            "title": "Marathon Squad",
            "group_id": &fx.group_id,
        }))
        .send(fx.router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = response.json();
    assert_eq!(conv.group_id.as_deref(), Some(fx.group_id.as_str()));
    assert_eq!(conv.title, "Marathon Squad");

    // The binding is persisted, not just echoed: re-reading the row returns it.
    let fetched = AxumTestRequest::get(&format!("/api/chat/conversations/{}", conv.id))
        .header("authorization", &fx.owner_auth)
        .send(fx.router)
        .await;
    assert_eq!(fetched.status_code(), StatusCode::OK);
    let stored: ConversationResponse = fetched.json();
    assert_eq!(stored.group_id.as_deref(), Some(fx.group_id.as_str()));
}

#[tokio::test]
async fn test_conversation_without_group_id_stays_unscoped() {
    let fx = Box::pin(setup()).await;

    let response = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "title": "Just me" }))
        .send(fx.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = response.json();
    assert_eq!(conv.group_id, None);
}

#[tokio::test]
async fn test_non_member_cannot_scope_a_conversation_to_the_group() {
    let fx = Box::pin(setup()).await;

    let response = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.outsider_auth)
        .json(&json!({
            "title": "Peeking at the squad",
            "group_id": &fx.group_id,
        }))
        .send(fx.router.clone())
        .await;

    // A membership refusal, not an auth one: the outsider is a real member of
    // the tenant, so a 401 here would mean the token never reached the check.
    assert_eq!(response.status_code(), StatusCode::FORBIDDEN);

    // The same caller can still open an unscoped conversation, so the refusal
    // is about the group binding and not about the caller's session.
    let unscoped = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.outsider_auth)
        .json(&json!({ "title": "My own chat" }))
        .send(fx.router)
        .await;
    assert_eq!(unscoped.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = unscoped.json();
    assert_eq!(conv.group_id, None);
}

/// A coach-facing agent the owner may see — they coach others.
async fn coach_facing_agent(fx: &Fixture) -> String {
    let repos = fx.res.agent.database.repositories();
    repos
        .users
        .set_coaches_others(fx.owner_id, true)
        .await
        .unwrap();
    let request: CreateAgentRequest = serde_json::from_value(json!({
        "title": "Roster Builder",
        "system_prompt": "Plan the coach's roster.",
        "category": "training",
        "tags": [Agent::COACH_TOOL_TAG]
    }))
    .unwrap();
    repos
        .agents
        .create(fx.owner_id, fx.shared_tid, &request)
        .await
        .unwrap()
        .id
        .to_string()
}

/// A room's turns reach every member, so a room conversation never binds a
/// coach-facing agent — not even when a coach creates it (carnet#910).
#[tokio::test]
async fn test_a_room_conversation_never_binds_a_coach_facing_agent() {
    let fx = Box::pin(setup()).await;
    let coach_agent = coach_facing_agent(&fx).await;

    let room = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "group_id": &fx.group_id, "agent_id": &coach_agent }))
        .send(fx.router.clone())
        .await;
    assert_eq!(room.status_code(), StatusCode::BAD_REQUEST);

    // The same coach binds it to a thread of their own.
    let personal = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "title": "Roster", "agent_id": &coach_agent }))
        .send(fx.router)
        .await;
    assert_eq!(personal.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = personal.json();
    assert_eq!(conv.agent_id.as_deref(), Some(coach_agent.as_str()));
}

/// An agent the caller cannot read is refused, not bound.
#[tokio::test]
async fn test_a_conversation_never_binds_an_unreadable_agent() {
    let fx = Box::pin(setup()).await;

    let response = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "title": "Ghost", "agent_id": Uuid::new_v4().to_string() }))
        .send(fx.router)
        .await;

    assert_eq!(response.status_code(), StatusCode::NOT_FOUND);
}
