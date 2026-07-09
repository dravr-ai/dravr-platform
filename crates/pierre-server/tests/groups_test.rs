// ABOUTME: Integration tests for group coaching REST API endpoints
// ABOUTME: Tests CRUD, membership, invites, authorization, and tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_coaches::build_coaches_router;
use pierre_routes_social::group_analytics::GroupAnalyticsRoutes;
use pierre_routes_social::GroupRoutes;

use axum::http::StatusCode;
use serde_json::{json, Value};

/// Assert response is success (200, 201, or 204)
fn assert_success(resp: &helpers::axum_test::AxumTestResponse, context: &str) {
    let s = resp.status_code();
    assert!(
        s == StatusCode::OK || s == StatusCode::CREATED || s == StatusCode::NO_CONTENT,
        "{context}: expected 2xx success, got {s}"
    );
}
use std::sync::Arc;

// ============================================================================
// Test Helpers
// ============================================================================

async fn create_test_coach(router: &axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/coaches")
        .header("authorization", auth)
        .json(&json!({"title":"Test Coach","system_prompt":"Test.","category":"training","tags":["run"]}))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json::<Value>()["id"].as_str().unwrap().to_owned()
}

async fn setup_single_user() -> (axum::Router, String, String, String) {
    setup_single_user_with("groupuser@test.com", "professional").await
}

/// Like [`setup_single_user`] but on an explicit billing `plan` so tier
/// enforcement (group coaching availability + member cap) can be exercised.
async fn setup_single_user_with(email: &str, plan: &str) -> (axum::Router, String, String, String) {
    let res = create_test_server_resources().await.unwrap();
    let (uid, u, _tid) = create_test_user_with_plan(&res.coach.database, email, plan)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&res, &u).await);
    // GroupAnalyticsRoutes ({stats,report,health}) mounts separately from GroupRoutes
    // in production (multitenant.rs); tests must mirror the composition root or
    // those three endpoints 404. See group_analytics.rs:54 — needs ToolRuntime +
    // SocialCtx + MiddlewareCtx (ServerContext satisfies all three).
    let router = build_coaches_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));
    let cid = create_test_coach(&router, &auth).await;
    (router, auth, uid.to_string(), cid)
}

async fn setup_two_users() -> (axum::Router, String, String, String, String, String) {
    let (router, a1, a2, u1, u2, cid, _a2_own) = setup_two_users_with_res().await;
    (router, a1, a2, u1, u2, cid)
}

/// Like [`setup_two_users`] but also returns a token for user2 that targets
/// user2's OWN tenant (last tuple element).
///
/// The shared-tenant `a2` token deliberately claims user1's tenant so both users
/// act inside one org, but user2 is never added to that tenant's `tenant_users`
/// — so `get_user_role(user2, shared_tenant)` is `None` and, under the default
/// `group_creation_policy = admins_only`, user2 cannot create a group there.
/// A test where user2 must *own* a group therefore creates it with `a2_own`, in
/// user2's own tenant where user2 is the owner (owner bypasses the policy). The
/// group-scoped invite endpoints exercised afterwards carry no tenant filter, so
/// the group-level IDOR assertion holds regardless of which tenant owns group B.
async fn setup_two_users_with_res() -> (axum::Router, String, String, String, String, String, String)
{
    let res = create_test_server_resources().await.unwrap();
    // Owner on Professional: the shared tenant must allow group coaching.
    let (u1id, u1, _t1) =
        create_test_user_with_plan(&res.coach.database, "groupowner@test.com", "professional")
            .await
            .unwrap();
    // Professional so user2's OWN tenant also enables group coaching (a starter
    // tenant would fail the tier gate on group creation, not the permission gate).
    let (u2id, u2, u2_own_tid) =
        create_test_user_with_plan(&res.coach.database, "groupmember@test.com", "professional")
            .await
            .unwrap();

    // Generate tokens. User1 uses their own tenant (owner).
    // User2 needs a token with user1's tenant so both are in the same org.
    let a1 = format!("Bearer {}", generate_test_token(&res, &u1).await);

    // For user2, generate a token with user1's tenant_id
    let repos = res.coach.database.repositories();
    let tenants = repos.tenants.list_for_user(u1id).await.unwrap();
    let shared_tid = tenants.first().unwrap().id;
    let a2 = format!(
        "Bearer {}",
        res.auth
            .auth_manager
            .generate_token_with_tenant(&u2, &res.auth.jwks_manager, Some(shared_tid.to_string()))
            .unwrap()
    );

    // user2's OWN-tenant token: user2 is the owner of this tenant, so it may
    // create groups there deterministically on both backends (owner short-circuits
    // check_create_group_permission before the admins_only policy is consulted).
    let a2_own = format!(
        "Bearer {}",
        res.auth
            .auth_manager
            .generate_token_with_tenant(&u2, &res.auth.jwks_manager, Some(u2_own_tid.to_string()))
            .unwrap()
    );

    let router = build_coaches_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));
    let cid = create_test_coach(&router, &a1).await;
    (
        router,
        a1,
        a2,
        u1id.to_string(),
        u2id.to_string(),
        cid,
        a2_own,
    )
}

/// Create a group and return (`group_id`, `invite_code`)
async fn create_group_with_invite(
    router: &axum::Router,
    auth_token: &str,
    coach_id: &str,
) -> (String, String) {
    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", auth_token)
        .json(&json!({
            "name": "Test Marathon Group",
            "description": "Training together",
            "coach_id": coach_id,
            "max_members": 10
        }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap().to_owned();

    // Create invite
    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", auth_token)
        .json(&json!({}))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let invite: Value = resp.json();
    let invite_code = invite["code"].as_str().unwrap().to_owned();

    (group_id, invite_code)
}

// ============================================================================
// Group CRUD Tests
// ============================================================================

#[tokio::test]
async fn test_create_group() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "My Running Club",
            "description": "Weekly runs together",
            "coach_id": &coach_id,
            "max_members": 15
        }))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let body: Value = resp.json();
    assert_eq!(body["name"], "My Running Club");
    assert_eq!(body["description"], "Weekly runs together");
    assert!(body["id"].as_str().is_some());
    assert_eq!(body["is_active"], true);
    // peer_data_sharing defaults to TRUE so individual /group consent
    // toggles surface peer data without an extra owner action. The
    // owner can still flip this off in Group Settings as a kill switch.
    assert_eq!(
        body["peer_data_sharing"], true,
        "REST POST should default peer_data_sharing=true (kill-switch off, individual consent gates) — matches messaging_group_bind auto-bind default"
    );
}

#[tokio::test]
async fn test_starter_plan_cannot_create_group() {
    // Starter tier has group coaching disabled (max_members_per_group == 0).
    let (router, auth, _user_id, coach_id) =
        setup_single_user_with("starteruser@test.com", "starter").await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Starter Club",
            "coach_id": &coach_id,
            "max_members": 5
        }))
        .send(router)
        .await;

    assert_eq!(
        resp.status_code(),
        StatusCode::FORBIDDEN,
        "Starter plan must be denied group creation"
    );
}

#[tokio::test]
async fn test_professional_clamps_max_members_to_tier_cap() {
    // Professional tier caps members per group at 10.
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Big Club",
            "coach_id": &coach_id,
            "max_members": 50
        }))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let body: Value = resp.json();
    assert_eq!(
        body["max_members"], 10,
        "requested 50 must be clamped to the Professional tier cap of 10"
    );
}

#[tokio::test]
async fn test_create_group_missing_name_fails() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "coach_id": &coach_id
        }))
        .send(router)
        .await;

    // Should fail with 400 or 422 for missing required field
    assert!(
        resp.status_code() == StatusCode::BAD_REQUEST
            || resp.status_code() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn test_list_my_groups() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    // Create two groups
    for name in &["Group A", "Group B"] {
        AxumTestRequest::post("/api/groups")
            .header("authorization", &auth)
            .json(&json!({
                "name": name,
                "coach_id": &coach_id
            }))
            .send(router.clone())
            .await;
    }

    let resp = AxumTestRequest::get("/api/groups")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    let groups = body["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 2);
}

#[tokio::test]
async fn test_get_group() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Detail Group",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;

    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert_eq!(body["name"], "Detail Group");
}

#[tokio::test]
async fn test_update_group() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Old Name",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .json(&json!({
            "name": "New Name",
            "description": "Updated description"
        }))
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert_eq!(body["name"], "New Name");
}

#[tokio::test]
async fn test_delete_group() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Delete Me",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;

    assert_success(&resp, "request");

    // Group should not appear in list anymore
    let resp = AxumTestRequest::get("/api/groups")
        .header("authorization", &auth)
        .send(router)
        .await;
    let body: Value = resp.json();
    let groups = body["groups"].as_array().unwrap();
    assert!(groups.is_empty());
}

// ============================================================================
// Membership Tests
// ============================================================================

#[tokio::test]
async fn test_owner_auto_added_as_member() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Owner Test",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0]["role"], "owner");
}

#[tokio::test]
async fn test_join_via_invite_code() {
    let (router, auth1, auth2, _user1_id, _user2_id, coach_id) = setup_two_users().await;

    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // User2 joins via invite
    let resp = AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    assert_success(&resp, "join group");

    // Verify member list has 2 members
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth1)
        .send(router)
        .await;
    let body: Value = resp.json();
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
}

#[tokio::test]
async fn test_cannot_join_twice() {
    let (router, auth1, auth2, _u1, _u2, coach_id) = setup_two_users().await;
    let (_group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // First join succeeds
    let resp = AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // Second join fails
    let resp = AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router)
        .await;
    assert_ne!(resp.status_code(), StatusCode::OK);
}

#[tokio::test]
async fn test_leave_group() {
    let (router, auth1, auth2, _u1, _u2, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // Join
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    // Leave
    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/leave"))
        .header("authorization", &auth2)
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // Member list should have 1 (just owner)
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth1)
        .send(router)
        .await;
    let body: Value = resp.json();
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 1);
}

#[tokio::test]
async fn test_remove_member_by_admin() {
    let (router, auth1, auth2, _u1, user2_id, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // User2 joins
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    // Owner removes user2
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}/members/{user2_id}"))
        .header("authorization", &auth1)
        .send(router)
        .await;

    assert_success(&resp, "request");
}

// ============================================================================
// Authorization Tests
// ============================================================================

#[tokio::test]
async fn test_member_cannot_update_group() {
    let (router, auth1, auth2, _u1, _u2, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // User2 joins as member
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    // Member tries to update — should fail
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth2)
        .json(&json!({ "name": "Hacked Name" }))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_member_cannot_remove_others() {
    let (router, auth1, auth2, user1_id, _u2, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // User2 joins
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    // Member tries to remove owner — should fail
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}/members/{user1_id}"))
        .header("authorization", &auth2)
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_group_admin_cannot_demote_owner() {
    // Regression (T3MP3ST F3 / CWE-285): a group admin (not the owner) must not be
    // able to change the OWNER's role. handle_remove_member already refuses to remove
    // the owner; handle_update_role skipped the same guard, so an admin could demote
    // the owner to member and seize effective control. This pins the guard.
    let (router, auth1, auth2, user1_id, user2_id, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // User2 joins as a member...
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    // ...and the owner promotes user2 to admin (an owner-only op — must succeed).
    let promote = AxumTestRequest::put(&format!("/api/groups/{group_id}/members/{user2_id}/role"))
        .header("authorization", &auth1)
        .json(&json!({ "role": "admin" }))
        .send(router.clone())
        .await;
    assert_success(&promote, "owner promotes member to admin");

    // The freshly-minted admin now tries to demote the owner to member — must 403.
    let demote = AxumTestRequest::put(&format!("/api/groups/{group_id}/members/{user1_id}/role"))
        .header("authorization", &auth2)
        .json(&json!({ "role": "member" }))
        .send(router)
        .await;
    assert_eq!(
        demote.status_code(),
        StatusCode::FORBIDDEN,
        "a group admin must not be able to change the owner's role"
    );
}

#[tokio::test]
async fn test_owner_can_demote_admin_to_member() {
    // The owner-protection guard must NOT block legitimate role management: the owner
    // can still demote a (non-owner) admin back to member.
    let (router, auth1, auth2, _u1, user2_id, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    assert_success(
        &AxumTestRequest::put(&format!("/api/groups/{group_id}/members/{user2_id}/role"))
            .header("authorization", &auth1)
            .json(&json!({ "role": "admin" }))
            .send(router.clone())
            .await,
        "owner promotes member to admin",
    );
    assert_success(
        &AxumTestRequest::put(&format!("/api/groups/{group_id}/members/{user2_id}/role"))
            .header("authorization", &auth1)
            .json(&json!({ "role": "member" }))
            .send(router)
            .await,
        "owner demotes admin back to member",
    );
}

#[tokio::test]
async fn test_unauthenticated_request_fails() {
    let (router, _auth, _user_id, _coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::get("/api/groups").send(router).await;

    assert_eq!(resp.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_owner_cannot_leave_group() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Owner Leave Test",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    // Owner tries to leave — should be forbidden
    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/leave"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);
}

// ============================================================================
// Invite Tests
// ============================================================================

#[tokio::test]
async fn test_create_invite() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Invite Test",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", &auth)
        .json(&json!({
            "expires_in_days": 7,
            "max_uses": 5
        }))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let invite: Value = resp.json();
    assert!(invite["code"].as_str().is_some());
    assert_eq!(invite["is_active"], true);
}

#[tokio::test]
async fn test_list_invites() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Invite List Test",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    // Create 2 invites
    for _ in 0..2 {
        AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
            .header("authorization", &auth)
            .json(&json!({}))
            .send(router.clone())
            .await;
    }

    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    let invites = body["invites"].as_array().unwrap();
    assert_eq!(invites.len(), 2);
}

#[tokio::test]
async fn test_deactivate_invite() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;
    let (group_id, _invite_code) = create_group_with_invite(&router, &auth, &coach_id).await;

    // Get invite ID
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    let body: Value = resp.json();
    let invite_id = body["invites"][0]["id"].as_str().unwrap();

    // Deactivate
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}/invites/{invite_id}"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
}

#[tokio::test]
async fn test_admin_cannot_deactivate_other_groups_invite() {
    // Regression (Phase 4 authz / CWE-639 IDOR): handle_deactivate_invite proves
    // the caller administers the `group_id` in the path via require_admin, but the
    // repo UPDATE only filtered on invite id — so an admin of group A could pass
    // their own group_id (passing require_admin) while targeting an invite that
    // belongs to group B, deactivating another group's invite. The repo now scopes
    // the update by group_id; a cross-group target must 404 (not found), and the
    // legitimate owner of the invite's group must still be able to deactivate it.
    let (router, auth1, _auth2, _u1, _u2, coach_id, auth2_own) = setup_two_users_with_res().await;

    // Group A owned/administered by user1 (in the shared tenant).
    let (group_a, _code_a) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // Group B owned/administered by user2. user2 is not a member of the shared
    // tenant, so it creates group B in its OWN tenant via `auth2_own` (owner ->
    // group creation always permitted). The invite endpoints below are group-
    // scoped with no tenant filter, so the cross-group IDOR check is unaffected
    // by group B living in a different tenant.
    let coach_id_2 = create_test_coach(&router, &auth2_own).await;
    let (group_b, _code_b) = create_group_with_invite(&router, &auth2_own, &coach_id_2).await;
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_b}/invites"))
        .header("authorization", &auth2_own)
        .send(router.clone())
        .await;
    let body: Value = resp.json();
    let invite_b_id = body["invites"][0]["id"].as_str().unwrap().to_owned();

    // Attack: admin of group A targets group B's invite via group A's path.
    // require_admin(group_a) passes, but the invite belongs to group_b → 404.
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_a}/invites/{invite_b_id}"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::NOT_FOUND,
        "an admin must not deactivate an invite belonging to a different group"
    );

    // The invite must still be usable — the cross-group deactivation must have
    // been a no-op, so group B's legitimate admin can still deactivate it.
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_b}/invites/{invite_b_id}"))
        .header("authorization", &auth2_own)
        .send(router)
        .await;
    assert_success(&resp, "owning group's admin deactivates its own invite");
}

#[tokio::test]
async fn test_join_with_invalid_code_fails() {
    let (router, auth, ..) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth)
        .json(&json!({ "invite_code": "INVALID123" }))
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Peer Sharing Tests
// ============================================================================

#[tokio::test]
async fn test_update_peer_sharing_consent() {
    let (router, auth1, auth2, _u1, _u2, coach_id) = setup_two_users().await;
    let (group_id, invite_code) = create_group_with_invite(&router, &auth1, &coach_id).await;

    // User2 joins
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router.clone())
        .await;

    // User2 updates peer sharing consent
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}/members/me/consent"))
        .header("authorization", &auth2)
        .json(&json!({ "consent": true }))
        .send(router)
        .await;

    assert_success(&resp, "request");
}

#[tokio::test]
async fn test_toggle_group_peer_sharing() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Peer Sharing Test",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    // Enable peer sharing
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .json(&json!({ "peer_data_sharing": true }))
        .send(router.clone())
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert_eq!(body["peer_data_sharing"], true);
}

// ============================================================================
// Stats Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_group_stats() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Stats Group",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/stats"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert!(body["stats"]["total_members"].is_number());
}

#[tokio::test]
async fn test_get_group_health_flags() {
    let (router, auth, _user_id, coach_id) = setup_single_user().await;

    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth)
        .json(&json!({
            "name": "Health Group",
            "coach_id": &coach_id
        }))
        .send(router.clone())
        .await;
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/health"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert!(body["flags"].is_array());
}

// ============================================================================
// E2E Flow Test
// ============================================================================

#[tokio::test]
async fn test_full_group_lifecycle() {
    let (router, auth1, auth2, _u1, user2_id, coach_id) = setup_two_users().await;

    // 1. Owner creates group
    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", &auth1)
        .json(&json!({
            "name": "Full Lifecycle Group",
            "description": "E2E test group",
            "coach_id": &coach_id,
            "max_members": 20
        }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let group: Value = resp.json();
    let group_id = group["id"].as_str().unwrap();

    // 2. Owner creates invite
    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", &auth1)
        .json(&json!({ "expires_in_days": 30 }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let invite: Value = resp.json();
    let code = invite["code"].as_str().unwrap();

    // 3. User2 joins via invite code
    let resp = AxumTestRequest::post("/api/groups/join")
        .header("authorization", &auth2)
        .json(&json!({ "invite_code": code }))
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // 4. Verify 2 members
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    let body: Value = resp.json();
    assert_eq!(body["members"].as_array().unwrap().len(), 2);

    // 5. Owner enables peer sharing
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth1)
        .json(&json!({ "peer_data_sharing": true }))
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // 6. Member opts into peer sharing
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}/members/me/consent"))
        .header("authorization", &auth2)
        .json(&json!({ "consent": true }))
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // 7. Check stats
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/stats"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // 8. Owner removes member
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}/members/{user2_id}"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    assert_success(&resp, "request");

    // 9. Verify 1 member remaining
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    let body: Value = resp.json();
    assert_eq!(body["members"].as_array().unwrap().len(), 1);

    // 10. Owner deletes group
    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth1)
        .send(router)
        .await;
    assert_success(&resp, "request");
}

/// Cross-tenant isolation for the group entity, encoding the documented v1
/// tenant model (see `crates/pierre-routes-social/src/groups.rs:1095`):
/// **athlete membership is cross-tenant by design**, but the group entity
/// itself (update/delete) is tenant-scoped (`WHERE tenant_id`).
///
/// Every other groups test uses [`setup_two_users`], which deliberately forces
/// both users into ONE tenant, so the suite never crossed the tenant boundary
/// despite the file's "tenant isolation" charter. This puts the two users in
/// DISTINCT tenants and asserts both halves of the rule.
#[tokio::test]
async fn cross_tenant_group_entity_isolation() {
    let res = create_test_server_resources().await.unwrap();

    // Owner in tenant A (professional so group coaching is available).
    let (_u1id, u1, _t1) =
        create_test_user_with_plan(&res.coach.database, "ct-owner@test.com", "professional")
            .await
            .unwrap();
    // Outsider in a DISTINCT tenant B (its own professional tenant).
    let (_u2id, u2, _t2) =
        create_test_user_with_plan(&res.coach.database, "ct-outsider@test.com", "professional")
            .await
            .unwrap();
    let a1 = format!("Bearer {}", generate_test_token(&res, &u1).await);
    let a2 = format!("Bearer {}", generate_test_token(&res, &u2).await);

    let router = build_coaches_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));

    let cid = create_test_coach(&router, &a1).await;
    let (group_id, invite_code) = create_group_with_invite(&router, &a1, &cid).await;

    // Tenant-scoped: an outsider in another tenant cannot UPDATE the group.
    let update = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &a2)
        .json(&json!({ "name": "Hijacked by tenant B" }))
        .send(router.clone())
        .await;
    assert!(
        !update.status_code().is_success(),
        "a user in another tenant must NOT update the group, got {}",
        update.status_code()
    );

    // Tenant-scoped: an outsider in another tenant cannot DELETE the group.
    let delete = AxumTestRequest::delete(&format!("/api/groups/{group_id}"))
        .header("authorization", &a2)
        .send(router.clone())
        .await;
    assert!(
        !delete.status_code().is_success(),
        "a user in another tenant must NOT delete the group, got {}",
        delete.status_code()
    );

    // The group survived both attempts: the owner can still read it.
    let get = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &a1)
        .send(router.clone())
        .await;
    assert_eq!(
        get.status_code(),
        StatusCode::OK,
        "owner must still see the group after the cross-tenant attempts"
    );

    // Cross-tenant by design: an athlete from another tenant MAY join via
    // invite. Asserting this locks in the intended behaviour so a later
    // over-broad "tenant-scope everything" change is caught here, not in prod.
    let join = AxumTestRequest::post("/api/groups/join")
        .header("authorization", &a2)
        .json(&json!({ "invite_code": invite_code }))
        .send(router)
        .await;
    assert_success(&join, "cross-tenant athlete join (allowed by design)");
}
