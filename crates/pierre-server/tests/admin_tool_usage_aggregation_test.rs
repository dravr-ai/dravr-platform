// ABOUTME: GET /admin/tool-usage aggregates llm_usage rows into per-tool invocation, turn and latency counts
// ABOUTME: Seeds real usage rows for a tenant owner and pins the breakdown, its ordering and the empty window

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{ConversationTurnId, InsertLlmUsage, Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::LlmConsumptionRoutes;
use serde_json::Value;
use serial_test::serial;
use uuid::Uuid;

/// A tenant owner — the role the endpoint admits — and a bearer token for them.
async fn owner_and_token(resources: &Arc<ServerContext>) -> (Uuid, TenantId, String) {
    let email = format!("tool-usage-{}@example.com", Uuid::new_v4());
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();
    let mut user = User::new(email.clone(), password_hash, Some("Owner".to_owned()));
    user.is_admin = true;
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
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
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    (user_id, tenant_id, format!("Bearer {token}"))
}

/// One LLM call of `turn` that invoked `tools`, with its latency.
async fn record_call(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    user_id: Uuid,
    turn: u128,
    tools: &[&str],
    latency: Option<i64>,
) {
    let tenant = tenant_id.to_string();
    let user = user_id.to_string();
    let tools_called = serde_json::to_string(tools).unwrap();
    resources
        .common
        .repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: &tenant,
            user_id: &user,
            conversation_id: None,
            turn_id: ConversationTurnId::from_uuid(Uuid::from_u128(turn)),
            provider: "google",
            model: "gemini",
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cached_write_tokens: 0,
            reasoning_tokens: 0,
            call_type: "chat",
            tool_calls_count: i64::try_from(tools.len()).unwrap(),
            tools_called: &tools_called,
            execution_time_ms: latency,
            cost_usd: 0.0,
            call_sequence: Some(1),
        })
        .await
        .unwrap();
}

async fn tool_usage(resources: &Arc<ServerContext>, auth: &str, days: u16) -> Value {
    let response = AxumTestRequest::get(&format!("/admin/tool-usage?days={days}"))
        .header("authorization", auth)
        .send(LlmConsumptionRoutes::routes(resources.clone()))
        .await;
    assert_eq!(response.status(), 200);
    response.json()
}

fn tool<'a>(body: &'a Value, name: &str) -> Option<&'a Value> {
    body["breakdown"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["tool_name"] == name)
}

#[tokio::test]
#[serial]
async fn aggregates_per_tool_counts_turns_and_latency() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = owner_and_token(&resources).await;

    // turn 1: two calls (discover_routes+weather @100, discover_routes @200)
    // turn 2: one call (discover_routes @300)
    let calls: [(u128, &[&str], i64); 3] = [
        (1, &["discover_routes", "get_weather_forecast"], 100),
        (1, &["discover_routes"], 200),
        (2, &["discover_routes"], 300),
    ];
    for (turn, tools, latency) in calls {
        record_call(&resources, tenant_id, user_id, turn, tools, Some(latency)).await;
    }

    let body = tool_usage(&resources, &auth, 30).await;

    assert_eq!(body["days"], 30);
    assert_eq!(body["summary"]["turns_with_tools"], 2);
    assert_eq!(body["summary"]["unique_tools"], 2);
    assert_eq!(body["summary"]["total_invocations"], 4);

    // discover_routes: 3 invocations across 2 turns, avg (100+200+300)/3.
    let discover = tool(&body, "discover_routes");
    assert!(
        discover.is_some_and(|d| d["invocation_count"] == 3
            && d["turn_count"] == 2
            && d["avg_latency_ms"] == 200),
        "discover_routes breakdown wrong: {discover:?}"
    );
    // get_weather_forecast: 1 invocation, 1 turn, avg 100.
    let weather = tool(&body, "get_weather_forecast");
    assert!(
        weather.is_some_and(|w| w["invocation_count"] == 1
            && w["turn_count"] == 1
            && w["avg_latency_ms"] == 100),
        "get_weather_forecast breakdown wrong: {weather:?}"
    );

    // Most-used tool sorts first.
    assert_eq!(body["breakdown"][0]["tool_name"], "discover_routes");
}

#[tokio::test]
#[serial]
async fn empty_rows_yield_empty_breakdown() {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, _tenant_id, auth) = owner_and_token(&resources).await;

    let body = tool_usage(&resources, &auth, 7).await;

    assert_eq!(body["summary"]["total_invocations"], 0);
    assert_eq!(body["summary"]["unique_tools"], 0);
    assert_eq!(body["summary"]["turns_with_tools"], 0);
    assert!(body["breakdown"].as_array().unwrap().is_empty());
    assert_eq!(body["days"], 7);
}
