// ABOUTME: Integration tests for human-agent attachment to coaching groups
// ABOUTME: Covers coach-invite redemption, the single-coach guard, member invites, and detach
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A human coach (a roster-managing user) joins a coaching group by redeeming
//! a `kind: "coach"` invite, which attaches them as the group's
//! `coach_user_id` instead of adding an athlete member. Redemption goes
//! through `GroupService`, the code `/group join` runs (its eligibility gate
//! is pinned by `chat_discover_group_commands_test`); the group, invite and
//! detach reads go through the real `GroupRoutes` router.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{
    create_test_server_resources, create_test_user_with_email, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::{CoachingGroup, CreateGroupRequest, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_groups::GroupRoutes;

use axum::http::StatusCode;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Store an AI agent persona for `owner_id` and return its id.
async fn create_test_agent(res: &Arc<ServerContext>, owner_id: Uuid, tenant: TenantId) -> String {
    let request: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Coach","system_prompt":"Test.","category":"training","tags":["run"]}),
    )
    .unwrap();
    res.common
        .repos
        .agents
        .create(owner_id, tenant, &request)
        .await
        .unwrap()
        .id
        .to_string()
}

/// Owner on Professional plus the wired router. Returns the shared resources
/// (for DB-level setup like `set_manages_roster`), the router, the owner's
/// auth header and id, the owner's tenant, and an AI agent persona id.
async fn setup() -> (
    Arc<ServerContext>,
    axum::Router,
    String,
    Uuid,
    TenantId,
    String,
) {
    let res = create_test_server_resources().await.unwrap();
    let (owner_id, owner, _t) =
        create_test_user_with_plan(&res.agent.database, "coachowner@test.com", "professional")
            .await
            .unwrap();
    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);
    let shared_tid = res
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let router = GroupRoutes::routes(Arc::clone(&res));
    let agent_persona = create_test_agent(&res, owner_id, shared_tid).await;
    (res, router, owner_auth, owner_id, shared_tid, agent_persona)
}

/// Create a roster-managing agent user and return (`user_id`, auth header).
/// `tenant` pins the token's active tenant (use the group's tenant for an
/// in-tenant agent, `None` falls back to the user's own tenant).
async fn make_coach_user(
    res: &Arc<ServerContext>,
    email: &str,
    tenant: Option<TenantId>,
) -> (Uuid, String) {
    let (uid, user) = create_test_user_with_email(&res.agent.database, email)
        .await
        .unwrap();
    res.agent
        .database
        .repositories()
        .users
        .set_manages_roster(uid, true)
        .await
        .unwrap();
    let token = match tenant {
        Some(tid) => res
            .auth
            .auth_manager
            .generate_token_with_tenant(&user, &res.auth.jwks_manager, Some(tid.to_string()))
            .unwrap(),
        None => generate_test_token(res, &user).await,
    };
    (uid, format!("Bearer {token}"))
}

/// Create a group owned by `owner_id` and return its id.
async fn create_group(
    res: &Arc<ServerContext>,
    owner_id: Uuid,
    tenant: TenantId,
    agent_persona: &str,
) -> String {
    let request = CreateGroupRequest {
        name: "Coached Group".to_owned(),
        description: None,
        agent_id: agent_persona.to_owned(),
        max_members: Some(10),
    };
    res.group_service()
        .create_group(&request, owner_id, tenant, 50)
        .await
        .unwrap()
        .id
        .to_string()
}

/// Create an invite of the given `kind` ("member" or "coach") and return its code.
async fn create_invite(
    router: &axum::Router,
    owner_auth: &str,
    group_id: &str,
    kind: &str,
) -> String {
    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", owner_auth)
        .json(&json!({ "kind": kind }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let invite: Value = resp.json();
    assert_eq!(invite["kind"], kind, "invite kind should round-trip");
    invite["code"].as_str().unwrap().to_owned()
}

/// Redeem a coach invite as `coach_uid`, the service call `/group join` makes.
async fn redeem_coach(
    res: &Arc<ServerContext>,
    coach_uid: Uuid,
    code: &str,
    tenant: TenantId,
) -> AppResult<CoachingGroup> {
    res.group_service()
        .redeem_coach_invite(code, coach_uid, tenant)
        .await
}

#[tokio::test]
async fn test_coach_invite_redemption_attaches_coach() {
    let (res, router, owner_auth, owner_id, shared_tid, persona) = setup().await;
    let group_id = create_group(&res, owner_id, shared_tid, &persona).await;
    let code = create_invite(&router, &owner_auth, &group_id, "coach").await;

    let (coach_uid, _coach_auth) =
        make_coach_user(&res, "humancoach@test.com", Some(shared_tid)).await;

    // Redeem attaches the coach and returns the group (not a member record).
    let group = redeem_coach(&res, coach_uid, &code, shared_tid)
        .await
        .unwrap();
    assert_eq!(group.coach_user_id, Some(coach_uid));

    // GET the group as the owner — the human coach is now attached.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(resp.json::<Value>()["coach_user_id"], coach_uid.to_string());

    // The agent is NOT counted as an athlete member.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let members: Value = resp.json();
    assert_eq!(
        members["total"], 1,
        "only the owner should be a member; the coach is not"
    );
}

#[tokio::test]
async fn test_remove_coach_detaches() {
    let (res, router, owner_auth, owner_id, shared_tid, persona) = setup().await;
    let group_id = create_group(&res, owner_id, shared_tid, &persona).await;
    let code = create_invite(&router, &owner_auth, &group_id, "coach").await;
    let (coach_uid, _coach_auth) =
        make_coach_user(&res, "detachcoach@test.com", Some(shared_tid)).await;
    redeem_coach(&res, coach_uid, &code, shared_tid)
        .await
        .unwrap();

    // Owner detaches the agent.
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}/coach"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    // coach_user_id is cleared.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert!(resp.json::<Value>()["coach_user_id"].is_null());
}

#[tokio::test]
async fn test_single_coach_guard() {
    let (res, router, owner_auth, owner_id, shared_tid, persona) = setup().await;
    let group_id = create_group(&res, owner_id, shared_tid, &persona).await;

    let (c1, _coach1) = make_coach_user(&res, "coach1@test.com", Some(shared_tid)).await;
    let code1 = create_invite(&router, &owner_auth, &group_id, "coach").await;
    redeem_coach(&res, c1, &code1, shared_tid).await.unwrap();

    // A second, different coach cannot attach while one is present.
    let (c2, _coach2) = make_coach_user(&res, "coach2@test.com", Some(shared_tid)).await;
    let code2 = create_invite(&router, &owner_auth, &group_id, "coach").await;
    let refusal = redeem_coach(&res, c2, &code2, shared_tid)
        .await
        .unwrap_err();
    assert_eq!(
        refusal.code,
        ErrorCode::InvalidInput,
        "a group already has a coach; a different coach must be rejected: {refusal}"
    );
}

#[tokio::test]
async fn test_member_invite_does_not_attach_coach() {
    let (res, router, owner_auth, owner_id, shared_tid, persona) = setup().await;
    let group_id = create_group(&res, owner_id, shared_tid, &persona).await;
    // Default (member) invite — kind omitted defaults to member.
    let code = create_invite(&router, &owner_auth, &group_id, "member").await;

    let (uid, _coach_auth) = make_coach_user(&res, "memberjoin@test.com", Some(shared_tid)).await;
    let member = res
        .group_service()
        .join_group(&code, uid, shared_tid)
        .await
        .unwrap();
    assert_eq!(member.user_id, uid);

    // Redeeming a member invite makes them a member, NOT the agent.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert!(
        resp.json::<Value>()["coach_user_id"].is_null(),
        "a member invite must never attach a human coach"
    );
}
