// ABOUTME: Integration tests for group-scoped chat conversations created over REST
// ABOUTME: Pins that group_id round-trips for a member and is refused for a non-member
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse};
use pierre_routes_coaches::build_coaches_router;
use pierre_routes_groups::GroupRoutes;

/// One router carrying the three surfaces this flow crosses: coaches (a group
/// needs a coach persona), groups (create + membership), and chat (the
/// conversation that carries `group_id`). Production mounts all three on the
/// same app; a test that mounts only chat cannot create the group it scopes to.
struct Fixture {
    router: axum::Router,
    /// Owner of the group, in the shared tenant.
    owner_auth: String,
    /// A second real user in the same tenant who never joins the group.
    outsider_auth: String,
    group_id: String,
}

async fn setup() -> Fixture {
    let res = create_test_server_resources().await.unwrap();

    let (owner_id, owner, _owner_tid) = create_test_user_with_plan(
        &res.coach.database,
        "groupchatowner@test.com",
        "professional",
    )
    .await
    .unwrap();
    let (outsider_id, outsider, _outsider_tid) = create_test_user_with_plan(
        &res.coach.database,
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
    let repos = res.coach.database.repositories();
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

    let router = build_coaches_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(ChatRoutes::routes(Arc::clone(&res)));

    let coach_resp = AxumTestRequest::post("/api/coaches")
        .header("authorization", &owner_auth)
        .json(&json!({
            "title": "Squad Coach",
            "system_prompt": "Coach the squad.",
            "category": "training",
            "tags": ["run"]
        }))
        .send(router.clone())
        .await;
    assert_eq!(coach_resp.status_code(), StatusCode::CREATED);
    let coach_id = coach_resp.json::<Value>()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let group_resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &owner_auth)
        .json(&json!({ "name": "Marathon Squad", "coach_id": &coach_id }))
        .send(router.clone())
        .await;
    assert_eq!(group_resp.status_code(), StatusCode::CREATED);
    let group_id = group_resp.json::<Value>()["id"]
        .as_str()
        .unwrap()
        .to_owned();

    Fixture {
        router,
        owner_auth,
        outsider_auth,
        group_id,
    }
}

#[tokio::test]
async fn test_conversation_created_with_group_id_carries_it_back() {
    let fx = setup().await;

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
    let fx = setup().await;

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
    let fx = setup().await;

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
