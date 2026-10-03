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
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::agents::{
    AgentCategory, AgentVisibility, CreateAgentRequest, CreateSystemAgentRequest,
};
use pierre_core::models::groups::UpdateGroupRequest;
use pierre_core::models::{CreateGroupRequest, GroupMember, GroupSummary};
use pierre_core::models::{TenantId, User};
use pierre_database::seed_models::SeedAgentTranslation;
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::build_agents_router;
use pierre_routes_groups::group_analytics::GroupAnalyticsRoutes;
use pierre_routes_groups::GroupRoutes;

use axum::http::StatusCode;
use serde_json::{json, Value};
use uuid::Uuid;

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

/// The fixture doors this file opens that have no REST route: storing an
/// agent, creating a group and redeeming an invite. Each goes through the
/// code the chat commands run (`AgentsRepository::create`,
/// `GroupService::create_group`, `GroupService::join_group`), acting as the
/// user and tenant the given bearer token names.
struct Doors {
    res: Arc<ServerContext>,
}

impl Doors {
    fn new(res: &Arc<ServerContext>) -> Self {
        Self {
            res: Arc::clone(res),
        }
    }

    /// The user and active tenant a `Bearer <jwt>` header acts as.
    fn caller(&self, auth: &str) -> (Uuid, TenantId) {
        let token = auth.strip_prefix("Bearer ").unwrap();
        let claims = self
            .res
            .auth
            .auth_manager
            .validate_token(token, &self.res.auth.jwks_manager)
            .unwrap();
        let tenant = claims
            .active_tenant_id
            .as_deref()
            .map(|t| TenantId::parse_str(t).unwrap())
            .expect("test tokens carry an active tenant");
        (Uuid::parse_str(&claims.sub).unwrap(), tenant)
    }

    async fn agent(&self, auth: &str) -> String {
        let (user_id, tenant_id) = self.caller(auth);
        let request: CreateAgentRequest = serde_json::from_value(
            json!({"title":"Test Coach","system_prompt":"Test.","category":"training","tags":["run"]}),
        )
        .unwrap();
        self.res
            .common
            .repos
            .agents
            .create(user_id, tenant_id, &request)
            .await
            .unwrap()
            .id
            .to_string()
    }

    /// Create a group as the caller, under the tenant plan's tier exactly as
    /// `/group create` creates it, and return it as `{"id": ...}`.
    async fn group(&self, auth: &str, body: Value) -> Value {
        let (user_id, tenant_id) = self.caller(auth);
        let request: CreateGroupRequest = serde_json::from_value(body).unwrap();
        let plan = self
            .res
            .common
            .repos
            .tenants
            .get_by_id(tenant_id)
            .await
            .unwrap()
            .plan;
        let group = self
            .res
            .group_service()
            .create_group(
                &request,
                user_id,
                tenant_id,
                tier_strategy_for(&plan).as_ref(),
            )
            .await
            .unwrap();
        json!({ "id": group.id.to_string() })
    }

    /// The groups the caller belongs to, as `/group` lists them.
    async fn groups_of(&self, auth: &str) -> Vec<GroupSummary> {
        let (user_id, _) = self.caller(auth);
        self.res
            .common
            .repos
            .groups
            .list_groups_for_user(user_id)
            .await
            .unwrap()
    }

    /// Redeem a member invite as the caller, under the group's own tenant.
    async fn join(&self, auth: &str, code: &str) -> AppResult<GroupMember> {
        let (user_id, _) = self.caller(auth);
        let invite = self
            .res
            .common
            .repos
            .groups
            .get_invite_by_code(code)
            .await?
            .ok_or_else(|| AppError::not_found("Invalid or expired invite code"))?;
        let group_tenant = TenantId::parse_str(&invite.tenant_id).unwrap();
        self.res
            .group_service()
            .join_group(code, user_id, group_tenant)
            .await
    }
}

async fn setup_single_user() -> (axum::Router, String, String, String, Doors) {
    Box::pin(setup_single_user_with("groupuser@test.com", "professional")).await
}

/// Like [`setup_single_user`] but on an explicit billing `plan` so tier
/// enforcement (group coaching availability + member cap) can be exercised.
async fn setup_single_user_with(
    email: &str,
    plan: &str,
) -> (axum::Router, String, String, String, Doors) {
    let res = create_test_server_resources().await.unwrap();
    let (uid, u, _tid) = create_test_user_with_plan(&res.agent.database, email, plan)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&res, &u).await);
    // GroupAnalyticsRoutes ({stats,report,health}) mounts separately from GroupRoutes
    // in production (multitenant.rs); tests must mirror the composition root or
    // those three endpoints 404. See group_analytics.rs:54 — needs ToolRuntime +
    // GroupsCtx + MiddlewareCtx (ServerContext satisfies all three).
    let router = build_agents_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));
    let doors = Doors::new(&res);
    let cid = doors.agent(&auth).await;
    (router, auth, uid.to_string(), cid, doors)
}

async fn setup_two_users() -> (axum::Router, String, String, String, String, String, Doors) {
    let (router, a1, a2, u1, u2, cid, _a2_own, doors) = Box::pin(setup_two_users_with_res()).await;
    (router, a1, a2, u1, u2, cid, doors)
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
#[allow(clippy::type_complexity)]
async fn setup_two_users_with_res() -> (
    axum::Router,
    String,
    String,
    String,
    String,
    String,
    String,
    Doors,
) {
    let res = create_test_server_resources().await.unwrap();
    // Owner on Professional: the shared tenant must allow group coaching.
    let (u1id, u1, _t1) =
        create_test_user_with_plan(&res.agent.database, "groupowner@test.com", "professional")
            .await
            .unwrap();
    // Professional so user2's OWN tenant also enables group coaching (a starter
    // tenant would fail the tier gate on group creation, not the permission gate).
    let (u2id, u2, u2_own_tid) =
        create_test_user_with_plan(&res.agent.database, "groupmember@test.com", "professional")
            .await
            .unwrap();

    // Generate tokens. User1 uses their own tenant (owner).
    // User2 needs a token with user1's tenant so both are in the same org.
    let a1 = format!("Bearer {}", generate_test_token(&res, &u1).await);

    // For user2, generate a token with user1's tenant_id
    let repos = res.agent.database.repositories();
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

    let router = build_agents_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));
    let doors = Doors::new(&res);
    let cid = doors.agent(&a1).await;
    (
        router,
        a1,
        a2,
        u1id.to_string(),
        u2id.to_string(),
        cid,
        a2_own,
        doors,
    )
}

/// Create a group and return (`group_id`, `invite_code`)
async fn create_group_with_invite(
    doors: &Doors,
    router: &axum::Router,
    auth_token: &str,
    agent_id: &str,
) -> (String, String) {
    let group = doors
        .group(
            auth_token,
            json!({
                "name": "Test Marathon Group",
                "description": "Training together",
                "agent_id": agent_id,
                "max_members": 10
            }),
        )
        .await;
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
async fn test_get_group() {
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Detail Group",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Old Name",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Delete Me",
                "agent_id": &agent_id
            }),
        )
        .await;
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;

    assert_success(&resp, "request");

    // Group should not appear in the owner's groups anymore
    assert!(doors.groups_of(&auth).await.is_empty());
}

// ============================================================================
// Membership Tests
// ============================================================================

#[tokio::test]
async fn test_owner_auto_added_as_member() {
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Owner Test",
                "agent_id": &agent_id
            }),
        )
        .await;
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
async fn test_leave_group() {
    let (router, auth1, auth2, _u1, _u2, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // Join
    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, auth1, auth2, _u1, user2_id, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // User2 joins
    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, auth1, auth2, _u1, _u2, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // User2 joins as member
    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, auth1, auth2, user1_id, _u2, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // User2 joins
    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, auth1, auth2, user1_id, user2_id, agent_id, doors) =
        Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // User2 joins as a member...
    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, auth1, auth2, _u1, user2_id, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, _auth, _user_id, _coach_id, _doors) = Box::pin(setup_single_user()).await;

    let resp = AxumTestRequest::get("/api/groups/permissions")
        .send(router)
        .await;

    assert_eq!(resp.status_code(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_owner_cannot_leave_group() {
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Owner Leave Test",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Invite Test",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Invite List Test",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;
    let (group_id, _invite_code) =
        create_group_with_invite(&doors, &router, &auth, &agent_id).await;

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
    let (router, auth1, _auth2, _u1, _u2, agent_id, auth2_own, doors) =
        Box::pin(setup_two_users_with_res()).await;

    // Group A owned/administered by user1 (in the shared tenant).
    let (group_a, _code_a) = create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // Group B owned/administered by user2. user2 is not a member of the shared
    // tenant, so it creates group B in its OWN tenant via `auth2_own` (owner ->
    // group creation always permitted). The invite endpoints below are group-
    // scoped with no tenant filter, so the cross-group IDOR check is unaffected
    // by group B living in a different tenant.
    let coach_id_2 = doors.agent(&auth2_own).await;
    let (group_b, _code_b) =
        create_group_with_invite(&doors, &router, &auth2_own, &coach_id_2).await;
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

// ============================================================================
// Peer Sharing Tests
// ============================================================================

#[tokio::test]
async fn test_update_peer_sharing_consent() {
    let (router, auth1, auth2, _u1, _u2, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    // User2 joins
    doors.join(&auth2, &invite_code).await.unwrap();

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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Peer Sharing Test",
                "agent_id": &agent_id
            }),
        )
        .await;
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

#[tokio::test]
async fn test_update_group_respond_mode_round_trips() {
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Respond Mode Test",
                "agent_id": &agent_id
            }),
        )
        .await;
    let group_id = group["id"].as_str().unwrap();

    // Flip to mentions-only via the same PUT the web settings form uses.
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .json(&json!({ "respond_mode": "mentions" }))
        .send(router.clone())
        .await;
    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert_eq!(body["respond_mode"], "mentions");

    // A fresh GET must see the persisted value — this is exactly the reload
    // path that regressed when GroupResponse dropped the field.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert_eq!(body["respond_mode"], "mentions");

    // An unrelated update must not clobber the stored mode (COALESCE path).
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .json(&json!({ "description": "unrelated change" }))
        .send(router.clone())
        .await;
    assert_success(&resp, "request");
    let body: Value = resp.json();
    assert_eq!(body["respond_mode"], "mentions");
}

// ============================================================================
// Stats Endpoint Tests
// ============================================================================

#[tokio::test]
async fn test_get_group_stats() {
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Stats Group",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth, _user_id, agent_id, doors) = Box::pin(setup_single_user()).await;

    let group = doors
        .group(
            &auth,
            json!({
                "name": "Health Group",
                "agent_id": &agent_id
            }),
        )
        .await;
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
    let (router, auth1, auth2, _u1, user2_id, agent_id, doors) = Box::pin(setup_two_users()).await;

    // 1. Owner creates group
    let group = doors
        .group(
            &auth1,
            json!({
                "name": "Full Lifecycle Group",
                "description": "E2E test group",
                "agent_id": &agent_id,
                "max_members": 20
            }),
        )
        .await;
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
    doors.join(&auth2, code).await.unwrap();

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
/// tenant model (see `crates/pierre-routes-groups/src/groups.rs:1095`):
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
        create_test_user_with_plan(&res.agent.database, "ct-owner@test.com", "professional")
            .await
            .unwrap();
    // Outsider in a DISTINCT tenant B (its own professional tenant).
    let (_u2id, u2, _t2) =
        create_test_user_with_plan(&res.agent.database, "ct-outsider@test.com", "professional")
            .await
            .unwrap();
    let a1 = format!("Bearer {}", generate_test_token(&res, &u1).await);
    let a2 = format!("Bearer {}", generate_test_token(&res, &u2).await);

    let router = build_agents_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)))
        .merge(GroupAnalyticsRoutes::routes(Arc::clone(&res)));

    let doors = Doors::new(&res);
    let cid = doors.agent(&a1).await;
    let (group_id, invite_code) = create_group_with_invite(&doors, &router, &a1, &cid).await;

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
    doors
        .join(&a2, &invite_code)
        .await
        .expect("cross-tenant athlete join (allowed by design)");
}

// ============================================================================
// Starter plan: a coach, one athlete and the agent per group
// ============================================================================

/// POST `/api/groups` as `auth` with the given agent, returning status + body.
async fn post_group(
    router: &axum::Router,
    auth: &str,
    agent_id: &str,
    name: &str,
) -> (StatusCode, Value) {
    let resp = AxumTestRequest::post("/api/groups")
        .header("authorization", auth)
        .json(&json!({ "name": name, "agent_id": agent_id }))
        .send(router.clone())
        .await;
    let status = resp.status_code();
    (status, resp.json())
}

/// A Starter athlete with their own tenant, as a `Bearer` header.
async fn starter_athlete(doors: &Doors, email: &str) -> String {
    let (_, user, _) = create_test_user_with_plan(&doors.res.agent.database, email, "starter")
        .await
        .unwrap();
    format!("Bearer {}", generate_test_token(&doors.res, &user).await)
}

#[tokio::test]
async fn test_starter_group_holds_its_coach_and_one_athlete() {
    let (router, auth, _coach_id, agent_id, doors) =
        Box::pin(setup_single_user_with("starter-coach@test.com", "starter")).await;

    let (status, group) = post_group(&router, &auth, &agent_id, "Marie").await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    // The coach (owner) and the athlete; the agent is the group's agent_id.
    assert_eq!(group["max_members"], 2);
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", &auth)
        .json(&json!({}))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let code = resp.json::<Value>()["code"].as_str().unwrap().to_owned();

    let athlete = starter_athlete(&doors, "starter-athlete@test.com").await;
    let joined = doors.join(&athlete, &code).await.unwrap();
    assert_eq!(joined.group_id.to_string(), group_id);

    let second = starter_athlete(&doors, "starter-second@test.com").await;
    let refusal = doors
        .join(&second, &code)
        .await
        .expect_err("a Starter group holds one athlete");
    assert_eq!(refusal.code, ErrorCode::InvalidInput);
    assert_eq!(refusal.message, "This group is full");
}

/// Group settings no longer change the member limit: a `max_members` in an
/// update body is ignored, so an owner cannot lift a group past its plan.
#[tokio::test]
async fn test_group_update_cannot_raise_the_member_limit() {
    let (router, auth, _coach_id, agent_id, _doors) = Box::pin(setup_single_user_with(
        "starter-settings@test.com",
        "starter",
    ))
    .await;

    let (status, group) = post_group(&router, &auth, &agent_id, "Marie").await;
    assert_eq!(status, StatusCode::CREATED, "{group}");
    let group_id = group["id"].as_str().unwrap();

    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth)
        .json(&json!({ "name": "Marie's training", "max_members": 50 }))
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let updated: Value = resp.json();
    assert_eq!(updated["name"], "Marie's training");
    assert_eq!(updated["max_members"], 2);
}

/// The group allowance comes from the tenant's plan, not the tier the
/// `GroupService` itself is built with (Professional, which allows 15): a
/// Starter coach makes a group per athlete for a 5–10 athlete cohort, so the
/// 10th group is granted and the 11th refused.
#[tokio::test]
async fn test_starter_coach_gets_a_group_per_athlete_up_to_ten() {
    let (router, auth, _coach_id, agent_id, _doors) =
        Box::pin(setup_single_user_with("starter-cohort@test.com", "starter")).await;

    for athlete in 1..=10 {
        let (status, body) =
            post_group(&router, &auth, &agent_id, &format!("Athlete {athlete}")).await;
        assert_eq!(status, StatusCode::CREATED, "group {athlete}: {body}");
    }

    let (status, body) = post_group(&router, &auth, &agent_id, "Athlete 11").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string().contains("Group limit reached (10)"),
        "the refusal names the plan's allowance: {body}"
    );
}

// ============================================================================
// Deleted-group access tests
// ============================================================================

/// `delete_group` archives the row rather than dropping it, and an invite
/// outlives that archive. Both redemption paths read the invite first, so
/// before this was fixed a link handed out before deletion still admitted a
/// stranger to a group its owner had already removed — against a confirm
/// dialog promising the group is gone and its members with it.
#[tokio::test]
async fn test_deleted_group_invite_cannot_be_redeemed() {
    let (router, auth1, auth2, _u1, _u2, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    assert_success(&resp, "delete group");

    // The code is still well-formed, unexpired and unused. Only the deletion
    // stands between it and a join.
    let refusal = doors.join(&auth2, &invite_code).await.unwrap_err();
    assert!(
        matches!(
            refusal.code,
            ErrorCode::ResourceNotFound | ErrorCode::InvalidInput
        ),
        "an invite for a deleted group must not admit anyone; got {refusal}"
    );

    // A refusal that still wrote the membership row would satisfy the status
    // assertion above. The roster endpoint answers only to members, so a client
    // error here is what proves no membership was created.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth2)
        .send(router)
        .await;
    assert!(
        resp.status_code().is_client_error(),
        "the refused join must leave the invitee outside the group; got {}",
        resp.status_code()
    );
}

/// Deleting a group releases its members, so a member who joined before the
/// delete loses access with it. Previously the membership row survived the
/// archive and the group stayed readable to everyone already in it.
#[tokio::test]
async fn test_delete_group_releases_its_members() {
    let (router, auth1, auth2, _u1, _u2, agent_id, doors) = Box::pin(setup_two_users()).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &auth1, &agent_id).await;

    doors
        .join(&auth2, &invite_code)
        .await
        .expect("join before delete");

    // The joiner can read the roster while the group is live — this is the
    // access the delete has to revoke, established before revoking it.
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth2)
        .send(router.clone())
        .await;
    assert_success(&resp, "member reads roster before delete");
    let body: Value = resp.json();
    assert_eq!(
        body["members"].as_array().unwrap().len(),
        2,
        "owner plus the joiner before the delete"
    );

    let resp = AxumTestRequest::delete(&format!("/api/groups/{group_id}"))
        .header("authorization", &auth1)
        .send(router.clone())
        .await;
    assert_success(&resp, "delete group");

    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &auth2)
        .send(router.clone())
        .await;
    assert!(
        resp.status_code().is_client_error(),
        "a released member must not keep reading a deleted group; got {}",
        resp.status_code()
    );

    // And it is gone from both sides' listings, not merely unreadable.
    for (who, auth) in [("owner", &auth1), ("joiner", &auth2)] {
        let groups = doors.groups_of(auth).await;
        assert!(
            groups.is_empty(),
            "{who} still lists the deleted group: {groups:?}"
        );
    }
}

// ============================================================================
// Weekly digest mode
// ============================================================================

/// An owner, a plain member and a human coach attached to the owner's group,
/// all acting in the owner's tenant. The coach is attached, not enrolled: they
/// hold no membership row. Returns the router, the three tokens and the group.
async fn setup_group_with_member_and_coach() -> (axum::Router, String, String, String, String) {
    let res = create_test_server_resources().await.unwrap();
    let db = &res.agent.database;
    let (owner_id, owner, _) =
        create_test_user_with_plan(db, "digest-owner@test.com", "professional")
            .await
            .unwrap();
    let (_, member, _) = create_test_user_with_plan(db, "digest-member@test.com", "professional")
        .await
        .unwrap();
    let (coach_id, coach, _) =
        create_test_user_with_plan(db, "digest-coach@test.com", "professional")
            .await
            .unwrap();
    let repos = db.repositories();
    let shared = repos.tenants.list_for_user(owner_id).await.unwrap()[0].id;
    let in_shared_tenant = |user: &User| {
        format!(
            "Bearer {}",
            res.auth
                .auth_manager
                .generate_token_with_tenant(user, &res.auth.jwks_manager, Some(shared.to_string()))
                .unwrap()
        )
    };
    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);
    let member_auth = in_shared_tenant(&member);
    let coach_auth = in_shared_tenant(&coach);

    let router = build_agents_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)));
    let doors = Doors::new(&res);
    let agent_id = doors.agent(&owner_auth).await;
    let (group_id, invite_code) =
        create_group_with_invite(&doors, &router, &owner_auth, &agent_id).await;
    doors.join(&member_auth, &invite_code).await.unwrap();
    assert!(repos
        .groups
        .set_group_coach_user(&group_id, Some(coach_id), shared)
        .await
        .unwrap());

    (router, owner_auth, member_auth, coach_auth, group_id)
}

async fn put_group(
    router: &axum::Router,
    auth: &str,
    group_id: &str,
    body: Value,
) -> (StatusCode, Value) {
    let resp = AxumTestRequest::put(&format!("/api/groups/{group_id}"))
        .header("authorization", auth)
        .json(&body)
        .send(router.clone())
        .await;
    let status = resp.status_code();
    let body = if status.is_success() {
        resp.json()
    } else {
        Value::Null
    };
    (status, body)
}

async fn get_group_as(router: &axum::Router, auth: &str, group_id: &str) -> Value {
    let resp = AxumTestRequest::get(&format!("/api/groups/{group_id}"))
        .header("authorization", auth)
        .send(router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json()
}

/// The owner sets each mode through the PUT the settings forms use; a fresh
/// GET reads it back, an unrelated update keeps it, and a value that is not
/// a mode is refused without touching the stored one.
#[tokio::test]
async fn test_owner_sets_the_digest_mode_and_it_round_trips() {
    let (router, owner, _member, _coach, group_id) =
        Box::pin(setup_group_with_member_and_coach()).await;

    for mode in ["chat", "managers", "off", "chat"] {
        let (status, body) =
            put_group(&router, &owner, &group_id, json!({ "digest_mode": mode })).await;
        assert_eq!(status, StatusCode::OK, "{mode}");
        assert_eq!(body["digest_mode"], mode);
        assert_eq!(
            get_group_as(&router, &owner, &group_id).await["digest_mode"],
            mode
        );
    }

    let (status, body) = put_group(
        &router,
        &owner,
        &group_id,
        json!({ "description": "unrelated" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["digest_mode"], "chat",
        "an unrelated update keeps the mode"
    );

    let (status, _) = put_group(
        &router,
        &owner,
        &group_id,
        json!({ "digest_mode": "weekly" }),
    )
    .await;
    assert!(
        status.is_client_error(),
        "an unknown mode is refused, got {status}"
    );
    assert_eq!(
        get_group_as(&router, &owner, &group_id).await["digest_mode"],
        "chat"
    );
}

/// The attached human coach may change where the digest goes — and nothing
/// else: a request naming any other field is refused whole, the digest part
/// included, and the group is left as it was.
#[tokio::test]
async fn test_attached_coach_may_change_only_the_digest_mode() {
    let (router, owner, _member, coach, group_id) =
        Box::pin(setup_group_with_member_and_coach()).await;

    let (status, body) = put_group(
        &router,
        &coach,
        &group_id,
        json!({ "digest_mode": "managers" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["digest_mode"], "managers");

    let (status, _) = put_group(
        &router,
        &coach,
        &group_id,
        json!({ "name": "Coach's Club" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the coach may not rename the group"
    );

    let (status, _) = put_group(
        &router,
        &coach,
        &group_id,
        json!({ "digest_mode": "chat", "respond_mode": "mentions" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a request carrying another field is refused whole"
    );

    let after = get_group_as(&router, &owner, &group_id).await;
    assert_eq!(after["name"], "Test Marathon Group");
    assert_eq!(after["respond_mode"], "all");
    assert_eq!(
        after["digest_mode"], "managers",
        "the refused request changed nothing"
    );
}

/// A plain member may not change the digest mode; promoted to admin, they may.
#[tokio::test]
async fn test_plain_member_is_refused_the_digest_mode_until_made_admin() {
    let (router, owner, member, _coach, group_id) =
        Box::pin(setup_group_with_member_and_coach()).await;

    let (status, _) = put_group(
        &router,
        &member,
        &group_id,
        json!({ "digest_mode": "chat" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        get_group_as(&router, &owner, &group_id).await["digest_mode"],
        "off"
    );

    let members: Value = AxumTestRequest::get(&format!("/api/groups/{group_id}/members"))
        .header("authorization", &owner)
        .send(router.clone())
        .await
        .json();
    let member_id = members["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "member")
        .expect("the member is listed")["user_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let promoted =
        AxumTestRequest::put(&format!("/api/groups/{group_id}/members/{member_id}/role"))
            .header("authorization", &owner)
            .json(&json!({ "role": "admin" }))
            .send(router.clone())
            .await;
    assert_success(&promoted, "promote to admin");

    let (status, body) = put_group(
        &router,
        &member,
        &group_id,
        json!({ "digest_mode": "chat" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["digest_mode"], "chat");
}

// ============================================================================
// Who is who: the agent, the human coach and the members by name
// ============================================================================

/// An owner whose group runs an agent with a catalogue handle and a French
/// title overlay, and a member who joined from their own, different tenant.
/// No human coach is attached yet.
struct RosterFixture {
    res: Arc<ServerContext>,
    router: axum::Router,
    owner_auth: String,
    member_auth: String,
    owner_id: Uuid,
    member_id: Uuid,
    coach_id: Uuid,
    owner_tenant: TenantId,
    group_id: String,
    created: Value,
}

async fn setup_roster() -> RosterFixture {
    let res = create_test_server_resources().await.unwrap();
    let db = &res.agent.database;
    let (owner_id, owner, owner_tenant) =
        create_test_user_with_plan(db, "roster-owner@test.com", "professional")
            .await
            .unwrap();
    let (member_id, member, member_tenant) =
        create_test_user_with_plan(db, "roster-member@test.com", "professional")
            .await
            .unwrap();
    let (coach_id, _, _) = create_test_user_with_plan(db, "roster-coach@test.com", "professional")
        .await
        .unwrap();
    assert_ne!(
        owner_tenant, member_tenant,
        "fixture precondition: the member acts from another tenant"
    );
    let repos = db.repositories();
    repos.users.update_locale(owner_id, "en").await.unwrap();
    repos.users.update_locale(member_id, "fr").await.unwrap();
    repos.users.update_locale(coach_id, "fr").await.unwrap();

    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);
    let member_auth = format!("Bearer {}", generate_test_token(&res, &member).await);
    let router = build_agents_router::<ServerContext>()
        .with_state(Arc::clone(&res))
        .merge(GroupRoutes::routes(Arc::clone(&res)));

    let doors = Doors::new(&res);
    let agent_id = doors.agent(&owner_auth).await;
    let handle = repos
        .store_listings
        .assign_catalogue_handle(&agent_id, owner_tenant)
        .await
        .unwrap();
    assert_eq!(handle, "test-coach");
    repos
        .seeder
        .seed_upsert_agent_translation(&SeedAgentTranslation {
            agent_id: agent_id.clone(),
            locale: "fr".to_owned(),
            title: Some("Coach de Test".to_owned()),
            description: None,
            purpose: None,
            instructions: None,
            source_sha: None,
            tags: None,
        })
        .await
        .unwrap();

    let group = doors
        .group(
            &owner_auth,
            json!({ "name": "Roster Club", "agent_id": agent_id, "max_members": 10 }),
        )
        .await;
    let group_id = group["id"].as_str().unwrap().to_owned();
    let created = get_group_as(&router, &owner_auth, &group_id).await;

    let invite: Value = AxumTestRequest::post(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", &owner_auth)
        .json(&json!({}))
        .send(router.clone())
        .await
        .json();
    doors
        .join(&member_auth, invite["code"].as_str().unwrap())
        .await
        .unwrap();

    RosterFixture {
        res,
        router,
        owner_auth,
        member_auth,
        owner_id,
        member_id,
        coach_id,
        owner_tenant,
        group_id,
        created,
    }
}

/// A plain member who joined from another tenant reads the group's agent by
/// its title in their own language and by its handle, and the human coach by
/// name — the agent is resolved in the group's tenant, where it runs, not in
/// the reader's. Every route that answers with a group names them the same
/// way: create, read, update and the coach's own list.
#[tokio::test]
async fn test_cross_tenant_member_reads_the_agent_and_coach_by_name() {
    let fx = Box::pin(setup_roster()).await;
    let repos = fx.res.agent.database.repositories();
    assert!(repos
        .groups
        .set_group_coach_user(&fx.group_id, Some(fx.coach_id), fx.owner_tenant)
        .await
        .unwrap());
    repos
        .users
        .update_display_name(fx.coach_id, "Karine Tremblay")
        .await
        .unwrap();

    assert_eq!(fx.created["agent_title"], "Test Coach");
    assert_eq!(fx.created["agent_handle"], "test-coach");

    let as_member = get_group_as(&fx.router, &fx.member_auth, &fx.group_id).await;
    assert_eq!(
        as_member["agent_title"], "Coach de Test",
        "the French member reads the agent's French title"
    );
    assert_eq!(as_member["agent_handle"], "test-coach");
    assert_eq!(as_member["coach_user_id"], fx.coach_id.to_string());
    assert_eq!(as_member["coach_display_name"], "Karine Tremblay");
    assert!(
        as_member.get("system_prompt").is_none(),
        "no agent internals reach the group body: {as_member}"
    );

    let as_owner = get_group_as(&fx.router, &fx.owner_auth, &fx.group_id).await;
    assert_eq!(
        as_owner["agent_title"], "Test Coach",
        "the English owner reads the canonical title"
    );
    assert_eq!(as_owner["coach_display_name"], "Karine Tremblay");

    let (status, updated) = put_group(
        &fx.router,
        &fx.owner_auth,
        &fx.group_id,
        json!({ "description": "Tuesday intervals" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(updated["agent_title"], "Test Coach");
    assert_eq!(updated["coach_display_name"], "Karine Tremblay");
}

/// Point the fixture's group at `agent_id` straight through the repository:
/// the create route does not check that an agent id belongs to the group's
/// tenant, so a row naming a foreign agent is a state the read must handle.
async fn repoint_group_agent(fx: &RosterFixture, agent_id: &str) {
    let updated = fx
        .res
        .agent
        .database
        .repositories()
        .groups
        .update_group(
            &fx.group_id,
            fx.owner_tenant,
            &UpdateGroupRequest {
                name: None,
                description: None,
                agent_id: Some(agent_id.to_owned()),
                peer_data_sharing: None,
                respond_mode: None,
                digest_mode: None,
                is_active: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        updated.map(|g| g.agent_id).as_deref(),
        Some(agent_id),
        "fixture precondition: the group now names the other agent"
    );
}

/// A group row naming a third tenant's private agent reads as an agent that
/// cannot be resolved: its title and handle stay private to that tenant,
/// while the rest of the group, the human coach included, still reads.
#[tokio::test]
async fn test_a_third_tenants_private_agent_is_never_named() {
    let fx = Box::pin(setup_roster()).await;
    let db = &fx.res.agent.database;
    let (_, stranger, stranger_tenant) =
        create_test_user_with_plan(db, "roster-stranger@test.com", "professional")
            .await
            .unwrap();
    assert_ne!(stranger_tenant, fx.owner_tenant, "fixture precondition");
    let stranger_auth = format!("Bearer {}", generate_test_token(&fx.res, &stranger).await);
    let private_agent = Doors::new(&fx.res).agent(&stranger_auth).await;
    repoint_group_agent(&fx, &private_agent).await;
    let repos = db.repositories();
    assert!(repos
        .groups
        .set_group_coach_user(&fx.group_id, Some(fx.coach_id), fx.owner_tenant)
        .await
        .unwrap());
    repos
        .users
        .update_display_name(fx.coach_id, "Karine Tremblay")
        .await
        .unwrap();

    let as_member = get_group_as(&fx.router, &fx.member_auth, &fx.group_id).await;
    assert_eq!(as_member["agent_id"], private_agent.as_str());
    assert_eq!(as_member["agent_title"], Value::Null);
    assert_eq!(as_member["agent_handle"], Value::Null);
    assert_eq!(as_member["coach_display_name"], "Karine Tremblay");
}

/// A system agent is shared across tenants, so a group running one is named
/// by that agent's title even though it was created in another tenant.
#[tokio::test]
async fn test_a_system_agent_from_another_tenant_is_named() {
    let fx = Box::pin(setup_roster()).await;
    let db = &fx.res.agent.database;
    let (admin_id, _, admin_tenant) =
        create_test_user_with_plan(db, "roster-system-admin@test.com", "professional")
            .await
            .unwrap();
    assert_ne!(admin_tenant, fx.owner_tenant, "fixture precondition");
    let system_agent = db
        .repositories()
        .agents
        .create_system_agent(
            admin_id,
            admin_tenant,
            &CreateSystemAgentRequest {
                title: "Shared Marathon Agent".to_owned(),
                description: None,
                system_prompt: "You coach the group.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    repoint_group_agent(&fx, &system_agent).await;

    let as_member = get_group_as(&fx.router, &fx.member_auth, &fx.group_id).await;
    assert_eq!(as_member["agent_id"], system_agent.as_str());
    assert_eq!(as_member["agent_title"], "Shared Marathon Agent");
    assert_eq!(as_member["coach_display_name"], Value::Null);
}

/// With no human coach the group names none; a coach whose display name is
/// blank is named by their email, as every other surface names a person.
#[tokio::test]
async fn test_coach_name_is_null_without_a_coach_and_falls_back_to_email() {
    let fx = Box::pin(setup_roster()).await;

    let without = get_group_as(&fx.router, &fx.member_auth, &fx.group_id).await;
    assert_eq!(without["coach_user_id"], Value::Null);
    assert_eq!(without["coach_display_name"], Value::Null);
    assert_eq!(
        without["agent_title"], "Coach de Test",
        "the agent is named whether or not a coach is attached"
    );

    let repos = fx.res.agent.database.repositories();
    repos
        .users
        .update_display_name(fx.coach_id, "   ")
        .await
        .unwrap();
    assert!(repos
        .groups
        .set_group_coach_user(&fx.group_id, Some(fx.coach_id), fx.owner_tenant)
        .await
        .unwrap());

    let with_blank = get_group_as(&fx.router, &fx.member_auth, &fx.group_id).await;
    assert_eq!(with_blank["coach_display_name"], "roster-coach@test.com");
}

/// The member list names each member by display name when they have one and
/// by email when they have none — never by email alone.
#[tokio::test]
async fn test_members_are_named_by_display_name_else_email() {
    let fx = Box::pin(setup_roster()).await;
    let repos = fx.res.agent.database.repositories();
    repos
        .users
        .update_display_name(fx.owner_id, "Olivia Owner")
        .await
        .unwrap();
    let mut member = repos.users.get_global(fx.member_id).await.unwrap().unwrap();
    member.display_name = None;
    repos.users.update(&member).await.unwrap();

    let body: Value = AxumTestRequest::get(&format!("/api/groups/{}/members", fx.group_id))
        .header("authorization", &fx.member_auth)
        .send(fx.router.clone())
        .await
        .json();
    let members = body["members"].as_array().unwrap();
    assert_eq!(members.len(), 2, "owner and member: {body}");
    let name_of = |user_id: Uuid| {
        members
            .iter()
            .find(|m| m["user_id"] == user_id.to_string())
            .unwrap_or_else(|| panic!("{user_id} is listed: {body}"))["display_name"]
            .clone()
    };
    assert_eq!(name_of(fx.owner_id), "Olivia Owner");
    assert_eq!(name_of(fx.member_id), "roster-member@test.com");
}
