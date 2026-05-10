// ABOUTME: Integration tests for group coaching REST API endpoints
// ABOUTME: Tests CRUD, membership, invites, authorization, and tenant isolation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user_with_email, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::routes::coaches::CoachesRoutes;
use pierre_mcp_server::routes::groups::GroupRoutes;

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
    let res = create_test_server_resources().await.unwrap();
    let (uid, u) = create_test_user_with_email(&res.database, "groupuser@test.com")
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&res, &u).await);
    let router =
        CoachesRoutes::routes(Arc::clone(&res)).merge(GroupRoutes::routes(Arc::clone(&res)));
    let cid = create_test_coach(&router, &auth).await;
    (router, auth, uid.to_string(), cid)
}

async fn setup_two_users() -> (axum::Router, String, String, String, String, String) {
    let res = create_test_server_resources().await.unwrap();
    let (u1id, u1) = create_test_user_with_email(&res.database, "groupowner@test.com")
        .await
        .unwrap();
    let (u2id, u2) = create_test_user_with_email(&res.database, "groupmember@test.com")
        .await
        .unwrap();

    // Generate tokens. User1 uses their own tenant (owner).
    // User2 needs a token with user1's tenant so both are in the same org.
    let a1 = format!("Bearer {}", generate_test_token(&res, &u1).await);

    // For user2, generate a token with user1's tenant_id
    let repos = res.database.repositories();
    let tenants = repos.tenants.list_for_user(u1id).await.unwrap();
    let shared_tid = tenants.first().unwrap().id;
    let a2 = format!(
        "Bearer {}",
        res.auth_manager
            .generate_token_with_tenant(&u2, &res.jwks_manager, Some(shared_tid.to_string()))
            .unwrap()
    );
    let router =
        CoachesRoutes::routes(Arc::clone(&res)).merge(GroupRoutes::routes(Arc::clone(&res)));
    let cid = create_test_coach(&router, &a1).await;
    (router, a1, a2, u1id.to_string(), u2id.to_string(), cid)
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
