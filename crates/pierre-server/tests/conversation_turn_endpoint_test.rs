// ABOUTME: Integration tests for GET /internal/conversation-turn/{turn_id} admin endpoint
// ABOUTME: Covers happy path, 404, auth failure, non-admin rejection, and cross-tenant isolation
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
use pierre_core::models::{ConversationTurnId, InsertLlmUsage, TenantId, TURN_SUMMARY_CALL_TYPE};
use pierre_mcp_server::{
    mcp::resources::ServerContext,
    models::{Tenant, User, UserStatus},
    permissions::UserRole,
    routes::llm_consumption::LlmConsumptionRoutes,
};
use serde_json::Value;
use serial_test::serial;
use std::sync::Arc;
use uuid::Uuid;

fn build_router(resources: Arc<ServerContext>) -> axum::Router {
    LlmConsumptionRoutes::routes(resources)
}

async fn create_admin_user_and_token(
    resources: &Arc<ServerContext>,
    email: &str,
) -> (Uuid, TenantId, String) {
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
    (user_id, tenant_id, format!("Bearer {token}"))
}

async fn create_regular_user_and_token(
    resources: &Arc<ServerContext>,
    email: &str,
) -> (TenantId, String) {
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

    // Demote the user from the default `owner` role that `tenants.create`
    // inserts — the endpoint rejects anything that is not admin or owner,
    // so this lets us exercise that branch.
    let pool = resources
        .database
        .sqlite_pool()
        .expect("test fixture runs against SQLite");
    sqlx::query("DELETE FROM tenant_users WHERE user_id = ?1 AND tenant_id = ?2")
        .bind(user_id.to_string())
        .bind(tenant_id)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at) \
         VALUES (?, ?, ?, 'member', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(tenant_id)
    .bind(user_id.to_string())
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .unwrap();

    let token = generate_test_token(resources, &user).await;
    (tenant_id, format!("Bearer {token}"))
}

/// Seed the `llm_usage` table with the exact shape the chat pipeline
/// produces for one turn: three per-call rows carrying real tokens and
/// latency, plus one turn-summary row with zero tokens and the
/// aggregate `tools_called` list and end-to-end `execution_time_ms`.
///
/// A fourth row under a different turn id exists to prove query
/// isolation. Returns the shared turn id so assertions can key off it.
async fn seed_turn(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
) -> ConversationTurnId {
    let tenant_str = tenant_id.to_string();
    let user_str = user_id.to_string();
    let turn = ConversationTurnId::new();

    // Three per-call rows — one per LLM call the tool loop would make.
    for i in 0..3i64 {
        let params = InsertLlmUsage {
            tenant_id: &tenant_str,
            user_id: &user_str,
            conversation_id: Some("conv-1"),
            turn_id: turn,
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 100 + i,
            completion_tokens: 20 + i,
            total_tokens: 120 + (2 * i),
            cached_tokens: 0,
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(500 + i),
            cost_usd: 0.0,
            call_sequence: None,
        };
        resources
            .repos
            .llm_usage
            .insert_llm_usage(&params)
            .await
            .unwrap();
    }

    // One turn-summary row — zero tokens, carries the aggregate
    // tools_called and end-to-end execution_time_ms.
    let summary = InsertLlmUsage {
        tenant_id: &tenant_str,
        user_id: &user_str,
        conversation_id: Some("conv-1"),
        turn_id: turn,
        provider: "gemini",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        cached_tokens: 0,
        call_type: TURN_SUMMARY_CALL_TYPE,
        tool_calls_count: 2,
        tools_called: "[\"get_activities\",\"get_training_load\"]",
        execution_time_ms: Some(3912),
        cost_usd: 0.0,
        call_sequence: None,
    };
    resources
        .repos
        .llm_usage
        .insert_llm_usage(&summary)
        .await
        .unwrap();

    // Noise row — different turn, same tenant/user — to prove
    // `find_llm_usage_by_turn_id` isolates on turn_id.
    let noise = InsertLlmUsage {
        tenant_id: &tenant_str,
        user_id: &user_str,
        conversation_id: Some("conv-1"),
        turn_id: ConversationTurnId::new(),
        provider: "google",
        model: "gemini-2.0-flash-exp",
        prompt_tokens: 999,
        completion_tokens: 999,
        total_tokens: 1998,
        cached_tokens: 0,
        call_type: "chat",
        tool_calls_count: 0,
        tools_called: "[]",
        execution_time_ms: Some(42),
        cost_usd: 0.0,
        call_sequence: None,
    };
    resources
        .repos
        .llm_usage
        .insert_llm_usage(&noise)
        .await
        .unwrap();

    turn
}

#[tokio::test]
#[serial]
async fn returns_full_turn_summary_for_admin() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (user_id, tenant_id, auth) =
        create_admin_user_and_token(&resources, "turn-admin@test.com").await;

    let turn = seed_turn(&resources, tenant_id, user_id).await;

    let router = build_router(resources);
    let response = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn}"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(response.status(), 200);

    let body: Value = response.json();
    assert_eq!(body["turn_id"], turn.to_string());
    assert_eq!(body["tenant_id"], tenant_id.to_string());
    assert_eq!(body["user_id"], user_id.to_string());
    assert_eq!(body["conversation_id"], "conv-1");

    // `llm_calls` only contains per-call rows — the turn-summary row
    // is excluded from the array.
    let llm_calls = body["llm_calls"].as_array().expect("llm_calls array");
    assert_eq!(
        llm_calls.len(),
        3,
        "summary row must not appear in llm_calls"
    );
    for call in llm_calls {
        assert_eq!(call["provider"], "google", "summary row leaked in");
    }

    // `tools_called` comes from the summary row's authoritative list.
    let tools: Vec<&str> = body["tools_called"]
        .as_array()
        .expect("tools_called array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(tools, vec!["get_activities", "get_training_load"]);

    // Aggregate token sums over per-call rows only:
    // Totals = (120 + 122 + 124) = 366.
    assert_eq!(body["total_tokens"].as_i64().unwrap(), 366);
    assert_eq!(body["total_prompt_tokens"].as_i64().unwrap(), 303);
    assert_eq!(body["total_completion_tokens"].as_i64().unwrap(), 63);
    // `total_latency_ms` comes from the summary row, which captures
    // end-to-end turn time including tool execution between calls —
    // NOT the sum of per-call latencies (500 + 501 + 502 = 1503).
    assert_eq!(body["total_latency_ms"].as_i64().unwrap(), 3912);

    Ok(())
}

#[tokio::test]
#[serial]
async fn returns_404_for_unknown_turn() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_, _, auth) = create_admin_user_and_token(&resources, "turn-admin-404@test.com").await;

    let unknown = ConversationTurnId::new();
    let router = build_router(resources);
    let response = AxumTestRequest::get(&format!("/internal/conversation-turn/{unknown}"))
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(response.status(), 404);
    Ok(())
}

#[tokio::test]
#[serial]
async fn returns_400_for_invalid_uuid() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_, _, auth) = create_admin_user_and_token(&resources, "turn-admin-400@test.com").await;

    let router = build_router(resources);
    let response = AxumTestRequest::get("/internal/conversation-turn/not-a-uuid")
        .header("authorization", &auth)
        .send(router)
        .await;

    assert_eq!(response.status(), 400);
    Ok(())
}

/// Simulates the end-to-end cross-repo data flow: canot generates the
/// turn id at its webhook boundary (`dravr_canot::ConversationTurnId`),
/// the platform bridges it into `pierre_core::ConversationTurnId` via
/// `From<CanotTurnId>`, writes `llm_usage` rows keyed to that id, and
/// the admin endpoint returns them when queried by the original id
/// rendered as a UUID string.
///
/// This is the integration contract the gist's "one turn id flows from
/// webhook to /internal/conversation-turn" requirement relies on.
#[tokio::test]
#[serial]
async fn canot_turn_id_bridges_to_platform_and_endpoint() -> Result<()> {
    use pierre_messaging::turn::ConversationTurnId as CanotTurnId;

    let resources = create_test_server_resources().await?;
    let (user, tenant, auth) =
        create_admin_user_and_token(&resources, "turn-bridge-admin@test.com").await;

    // Canot-side id that the webhook adapter would have stamped onto
    // `IncomingMessage.turn_id`.
    let canot_id = CanotTurnId::new();

    // Platform bridges via the `From<CanotTurnId>` impl — what
    // messaging_ingress does when building `PendingDispatch`.
    let platform_id: ConversationTurnId = canot_id.into();

    // Seed one per-call row + one summary row keyed to `platform_id`.
    let tenant_str = tenant.to_string();
    let user_str = user.to_string();
    resources
        .repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: &tenant_str,
            user_id: &user_str,
            conversation_id: Some("conv-bridge"),
            turn_id: platform_id,
            provider: "google",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 77,
            completion_tokens: 11,
            total_tokens: 88,
            cached_tokens: 0,
            call_type: "chat",
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(123),
            cost_usd: 0.0,
            call_sequence: None,
        })
        .await?;
    resources
        .repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: &tenant_str,
            user_id: &user_str,
            conversation_id: Some("conv-bridge"),
            turn_id: platform_id,
            provider: "gemini",
            model: "gemini-2.0-flash-exp",
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            call_type: TURN_SUMMARY_CALL_TYPE,
            tool_calls_count: 0,
            tools_called: "[]",
            execution_time_ms: Some(150),
            cost_usd: 0.0,
            call_sequence: None,
        })
        .await?;

    // Query the endpoint using the canot UUID rendered as a string —
    // the wire shape an external QA/on-call tool would send.
    let router = build_router(resources);
    let response = AxumTestRequest::get(&format!(
        "/internal/conversation-turn/{}",
        canot_id.as_uuid()
    ))
    .header("authorization", &auth)
    .send(router)
    .await;

    assert_eq!(response.status(), 200);
    let body: Value = response.json::<Value>();
    assert_eq!(body["turn_id"], canot_id.as_uuid().to_string());
    assert_eq!(body["llm_calls"].as_array().unwrap().len(), 1);
    assert_eq!(body["total_tokens"].as_u64().unwrap(), 88);
    Ok(())
}

/// Nil UUID is the pre-migration sentinel for `llm_usage.turn_id`;
/// returning the lumped set would be misleading, so the handler must
/// reject it explicitly with 400.
#[tokio::test]
#[serial]
async fn returns_400_for_nil_uuid() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (_, _, auth) = create_admin_user_and_token(&resources, "turn-admin-nil@test.com").await;

    let router = build_router(resources);
    let response =
        AxumTestRequest::get("/internal/conversation-turn/00000000-0000-0000-0000-000000000000")
            .header("authorization", &auth)
            .send(router)
            .await;

    assert_eq!(response.status(), 400);
    Ok(())
}

#[tokio::test]
#[serial]
async fn rejects_non_admin_caller() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (admin_user, admin_tenant, admin_auth) =
        create_admin_user_and_token(&resources, "turn-admin-regular@test.com").await;
    let (_, regular_auth) =
        create_regular_user_and_token(&resources, "turn-regular@test.com").await;

    // Admin seeds a turn so there is data to query.
    let turn = seed_turn(&resources, admin_tenant, admin_user).await;

    let router = build_router(resources);

    // Regular caller is denied — not an admin/owner of their tenant.
    let response = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn}"))
        .header("authorization", &regular_auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status(), 401);

    // Admin caller still succeeds as a sanity check that the fixture isn't broken.
    let ok = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn}"))
        .header("authorization", &admin_auth)
        .send(router)
        .await;
    assert_eq!(ok.status(), 200);

    Ok(())
}

#[tokio::test]
#[serial]
async fn rejects_cross_tenant_lookup() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (user_a, tenant_a, _auth_a) =
        create_admin_user_and_token(&resources, "turn-tenant-a@test.com").await;
    let (_user_b, _tenant_b, auth_b) =
        create_admin_user_and_token(&resources, "turn-tenant-b@test.com").await;

    // Seed a turn in tenant A.
    let turn = seed_turn(&resources, tenant_a, user_a).await;

    // Admin of tenant B asks for it.
    let router = build_router(resources);
    let response = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn}"))
        .header("authorization", &auth_b)
        .send(router)
        .await;

    assert_eq!(
        response.status(),
        401,
        "admin of tenant B must not see a turn from tenant A"
    );

    Ok(())
}

#[tokio::test]
#[serial]
async fn requires_authentication() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let turn = ConversationTurnId::new();

    let router = build_router(resources);
    let response = AxumTestRequest::get(&format!("/internal/conversation-turn/{turn}"))
        .send(router)
        .await;

    assert!(
        (400..500).contains(&response.status()),
        "unauthenticated request should fail, got {}",
        response.status()
    );

    Ok(())
}
