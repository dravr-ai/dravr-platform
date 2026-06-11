// ABOUTME: Integration tests for human-coach attachment to coaching groups
// ABOUTME: Covers coach-invite redemption, eligibility + cross-tenant gates, detach, and listing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A human coach (a roster-managing user) joins a coaching group by redeeming
//! a `kind: "coach"` invite, which attaches them as the group's
//! `coach_user_id` instead of adding an athlete member. These tests exercise
//! the REST surface end to end against the real `GroupRoutes` router:
//! the happy-path redemption, the `manages_roster` eligibility gate, the v1
//! same-tenant rule, the single-coach guard, detach, and the "groups I coach"
//! listing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{
    create_test_server_resources, create_test_user_with_email, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_coaches::build_coaches_router;
use pierre_routes_social::GroupRoutes;

use axum::http::StatusCode;
use serde_json::{json, Value};
use std::sync::Arc;
use uuid::Uuid;

/// Create an AI coach persona via the REST API and return its id.
async fn create_test_coach(router: &axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/coaches")
        .header("authorization", auth)
        .json(
            &json!({"title":"Coach","system_prompt":"Test.","category":"training","tags":["run"]}),
        )
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json::<Value>()["id"].as_str().unwrap().to_owned()
}

/// Owner on Professional plus the wired router. Returns the shared resources
/// (for DB-level setup like `set_manages_roster`), the router, the owner's
/// auth header, the owner's tenant, and an AI coach persona id.
async fn setup() -> (Arc<ServerContext>, axum::Router, String, TenantId, String) {
    let res = create_test_server_resources().await.unwrap();
    let (owner_id, owner, _t) =
        create_test_user_with_plan(&res.coach.database, "coachowner@test.com", "professional")
            .await
            .unwrap();
    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);
    let shared_tid = res
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    let router = build_coaches_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)));
    let coach_persona = create_test_coach(&router, &owner_auth).await;
    (res, router, owner_auth, shared_tid, coach_persona)
}

/// Create a roster-managing coach user and return (`user_id`, auth header).
/// `tenant` pins the token's active tenant (use the group's tenant for an
/// in-tenant coach, `None` falls back to the user's own tenant).
async fn make_coach_user(
    res: &Arc<ServerContext>,
    email: &str,
    tenant: Option<TenantId>,
) -> (Uuid, String) {
    let (uid, user) = create_test_user_with_email(&res.coach.database, email)
        .await
        .unwrap();
    res.coach
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

/// Create a group owned by `owner_auth` and return its id.
async fn create_group(router: &axum::Router, owner_auth: &str, coach_persona: &str) -> String {
    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", owner_auth)
        .json(&json!({
            "name": "Coached Group",
            "coach_id": coach_persona,
            "max_members": 10
        }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json::<Value>()["id"].as_str().unwrap().to_owned()
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

async fn redeem(
    router: &axum::Router,
    auth: &str,
    code: &str,
) -> helpers::axum_test::AxumTestResponse {
    AxumTestRequest::post("/api/groups/join")
        .header("authorization", auth)
        .json(&json!({ "invite_code": code }))
        .send(router.clone())
        .await
}

#[tokio::test]
async fn test_coach_invite_redemption_attaches_coach() {
    let (res, router, owner_auth, shared_tid, persona) = setup().await;
    let group_id = create_group(&router, &owner_auth, &persona).await;
    let code = create_invite(&router, &owner_auth, &group_id, "coach").await;

    let (coach_uid, coach_auth) =
        make_coach_user(&res, "humancoach@test.com", Some(shared_tid)).await;

    // Redeem attaches the coach and returns the group (not a member record).
    let resp = redeem(&router, &coach_auth, &code).await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let body: Value = resp.json();
    assert_eq!(body["coach_user_id"], coach_uid.to_string());

    // GET the group as the owner — the human coach is now attached.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(resp.json::<Value>()["coach_user_id"], coach_uid.to_string());

    // The coach sees the group in their "groups I coach" list.
    let resp = AxumTestRequest::get("/api/groups/coached")
        .header("authorization", &coach_auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: Value = resp.json();
    assert_eq!(body["total"], 1);
    assert_eq!(body["groups"][0]["id"], group_id);

    // The coach is NOT counted as an athlete member.
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
async fn test_non_coach_cannot_redeem_coach_invite() {
    let (res, router, owner_auth, shared_tid, persona) = setup().await;
    let group_id = create_group(&router, &owner_auth, &persona).await;
    let code = create_invite(&router, &owner_auth, &group_id, "coach").await;

    // A regular (non-roster) user in the same tenant.
    let (_uid, user) = create_test_user_with_email(&res.coach.database, "notacoach@test.com")
        .await
        .unwrap();
    let token = res
        .auth
        .auth_manager
        .generate_token_with_tenant(&user, &res.auth.jwks_manager, Some(shared_tid.to_string()))
        .unwrap();
    let auth = format!("Bearer {token}");

    let resp = redeem(&router, &auth, &code).await;
    assert_eq!(
        resp.status_code(),
        StatusCode::FORBIDDEN,
        "a user without manages_roster must not attach as coach"
    );
}

#[tokio::test]
async fn test_cross_tenant_coach_rejected() {
    let (res, router, owner_auth, _shared_tid, persona) = setup().await;
    let group_id = create_group(&router, &owner_auth, &persona).await;
    let code = create_invite(&router, &owner_auth, &group_id, "coach").await;

    // A roster-managing coach in a DIFFERENT tenant (own professional tenant).
    let (other_uid, other_user, _other_tenant) =
        create_test_user_with_plan(&res.coach.database, "othercoach@test.com", "professional")
            .await
            .unwrap();
    res.coach
        .database
        .repositories()
        .users
        .set_manages_roster(other_uid, true)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&res, &other_user).await);

    let resp = redeem(&router, &auth, &code).await;
    assert_eq!(
        resp.status_code(),
        StatusCode::FORBIDDEN,
        "a coach outside the group's tenant must be rejected (v1 same-tenant rule)"
    );
}

#[tokio::test]
async fn test_remove_coach_detaches() {
    let (res, router, owner_auth, shared_tid, persona) = setup().await;
    let group_id = create_group(&router, &owner_auth, &persona).await;
    let code = create_invite(&router, &owner_auth, &group_id, "coach").await;
    let (_coach_uid, coach_auth) =
        make_coach_user(&res, "detachcoach@test.com", Some(shared_tid)).await;
    assert_eq!(
        redeem(&router, &coach_auth, &code).await.status_code(),
        StatusCode::CREATED
    );

    // Owner detaches the coach.
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
    let (res, router, owner_auth, shared_tid, persona) = setup().await;
    let group_id = create_group(&router, &owner_auth, &persona).await;

    let (_c1, coach1) = make_coach_user(&res, "coach1@test.com", Some(shared_tid)).await;
    let code1 = create_invite(&router, &owner_auth, &group_id, "coach").await;
    assert_eq!(
        redeem(&router, &coach1, &code1).await.status_code(),
        StatusCode::CREATED
    );

    // A second, different coach cannot attach while one is present.
    let (_c2, coach2) = make_coach_user(&res, "coach2@test.com", Some(shared_tid)).await;
    let code2 = create_invite(&router, &owner_auth, &group_id, "coach").await;
    let resp = redeem(&router, &coach2, &code2).await;
    assert_eq!(
        resp.status_code(),
        StatusCode::BAD_REQUEST,
        "a group already has a coach; a different coach must be rejected"
    );
}

#[tokio::test]
async fn test_member_invite_does_not_attach_coach() {
    let (res, router, owner_auth, shared_tid, persona) = setup().await;
    let group_id = create_group(&router, &owner_auth, &persona).await;
    // Default (member) invite — kind omitted defaults to member.
    let code = create_invite(&router, &owner_auth, &group_id, "member").await;

    let (_uid, coach_auth) = make_coach_user(&res, "memberjoin@test.com", Some(shared_tid)).await;
    let resp = redeem(&router, &coach_auth, &code).await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);

    // Redeeming a member invite makes them a member, NOT the coach.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &owner_auth)
        .send(router.clone())
        .await;
    assert!(
        resp.json::<Value>()["coach_user_id"].is_null(),
        "a member invite must never attach a human coach"
    );
}
