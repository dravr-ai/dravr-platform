// ABOUTME: The trust label an MCP or A2A tool result carries in _meta: tier, source class, sensitive, untrusted (owl phase 1)
// ABOUTME: Drives /mcp, /a2a and the executor with stand-in providers; Dravr's own agent and a result serving nothing get none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::tier_consent_test::allow_outside_memory;
use crate::transport_gate_e2e_test::api_key;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::constants::oauth_providers::{INTERVALS_ICU, USER_PROVIDED_PROVIDERS, WAHOO};
use pierre_core::models::{
    Activity, ActivityBuilder, ConnectionType, SportType, TenantId, User, UserOAuthToken,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_mcp_server::context::ServerContext;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_providers::core::{FitnessProvider, ProviderConfig};
use pierre_providers::spi::ProviderBundle;
use pierre_tool_runtime::protocol::trust_labels::{labels_of, TRUST_LABELS_META_KEY};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user_with_plan};
use crate::helpers::stand_in_provider::{Account, StandIn};

/// One session the stand-in under `provider` answers with.
fn session(provider: &str) -> Activity {
    ActivityBuilder::new(
        format!("{provider}-run"),
        "Riverside Loop",
        SportType::Run,
        Timestamp::now() - Elapsed::days(1),
        3_600,
        provider,
    )
    .distance_meters(10_000.0)
    .average_heart_rate(148)
    .build()
}

fn wahoo_account(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Account {
        config,
        provider: WAHOO,
        activities: vec![session(WAHOO)],
    })
}

fn intervals_account(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Account {
        config,
        provider: INTERVALS_ICU,
        activities: vec![session(INTERVALS_ICU)],
    })
}

/// The test server, Wahoo (a device's own connection) and intervals.icu (a
/// coaching platform) stood in by accounts that answer from memory.
async fn server() -> Arc<ServerContext> {
    common::init_server_config();
    let base = create_test_server_resources().await.unwrap();
    let mut registry = common::offline_provider_registry();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(StandIn { name: WAHOO }),
        wahoo_account,
    ));
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(StandIn {
            name: INTERVALS_ICU,
        }),
        intervals_account,
    ));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

struct Rider {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
}

async fn athlete(resources: &ServerContext) -> Rider {
    let email = format!("labels-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    Rider {
        user,
        user_id,
        tenant,
    }
}

/// Connect `provider` for the athlete: a live token and a connection row.
async fn connect(resources: &ServerContext, athlete: &Rider, provider: &str) {
    let repos = resources.agent.database.repositories();
    let token = UserOAuthToken::new(
        athlete.user_id,
        athlete.tenant.to_string(),
        provider.to_owned(),
        format!("{provider}-access"),
        Some(format!("{provider}-refresh")),
        Some(Timestamp::now() + Elapsed::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            provider,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
}

/// Hold `activities` in the athlete's cache under `provider`.
async fn hold(resources: &ServerContext, athlete: &Rider, provider: &str, activities: &[Activity]) {
    resources
        .agent
        .database
        .repositories()
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, provider, activities)
        .await
        .unwrap();
}

/// Run `tool` for the athlete on an executor bound to `transport`, built the
/// way an MCP or A2A entry point builds it when `caller_writes_reply`.
async fn call(
    resources: &Arc<ServerContext>,
    athlete: &Rider,
    transport: Transport,
    caller_writes_reply: bool,
    tool: &str,
    parameters: Value,
) -> UniversalResponse {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(transport);
    let executor = if caller_writes_reply {
        executor.with_caller_writing_reply()
    } else {
        executor
    };
    executor
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool answers")
}

/// The label an MCP caller's call of `tool` carries.
async fn label(
    resources: &Arc<ServerContext>,
    athlete: &Rider,
    tool: &str,
    parameters: Value,
) -> Option<Value> {
    let response = call(
        resources,
        athlete,
        Transport::McpHttp,
        true,
        tool,
        parameters,
    )
    .await;
    assert!(response.success, "{tool}: {:?}", response.error);
    labels_of(&response).cloned()
}

fn activities_from(provider: &str) -> Value {
    json!({ "provider": provider, "limit": 10, "mode": "summary" })
}

async fn post(app: &Router, uri: &str, authorization: &str, body: &Value) -> Value {
    let response = app
        .clone()
        .oneshot(
            Request::post(uri)
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("A2A-Version", "1.0")
                .header("authorization", authorization)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{uri} answers");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// The label a device's sessions, named by the athlete, carry: T1 for their
/// start coordinates, untrusted for their names.
fn device_sessions_label() -> Value {
    json!({
        "v": 1,
        "tier": "T1",
        "source_class": ["device"],
        "sensitive": false,
        "untrusted": true
    })
}

// ============================================================================
// Over the wire: /mcp and /a2a
// ============================================================================

#[tokio::test]
async fn an_mcp_result_carries_its_trust_label_in_meta() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let token = format!(
        "Bearer {}",
        common::generate_delegated_test_token(&resources, &rider.user).await
    );

    let body = post(
        &app,
        "/mcp",
        &token,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_activities", "arguments": activities_from(WAHOO) }
        }),
    )
    .await;

    let result = &body["result"];
    assert!(
        result["structuredContent"]
            .to_string()
            .contains("Riverside Loop"),
        "the session reached the client: {body}"
    );
    assert_eq!(
        result["_meta"][TRUST_LABELS_META_KEY],
        device_sessions_label(),
        "{body}"
    );
    assert!(
        result["structuredContent"].get("_meta").is_none()
            && !result["structuredContent"]
                .to_string()
                .contains(TRUST_LABELS_META_KEY),
        "the label is for the host, never in the payload a model reads: {body}"
    );
}

#[tokio::test]
async fn an_a2a_tool_artifact_carries_the_same_label_in_its_metadata() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;
    resources
        .a2a
        .a2a_client_manager
        .register_client(
            pierre_a2a::ClientRegistrationRequest {
                name: format!("labels-agent-{}", rider.user_id),
                description: "an agent writing its own replies".into(),
                capabilities: vec!["fitness-data-analysis".into()],
                redirect_uris: vec![],
                contact_email: "agent@example.com".into(),
            },
            rider.user_id,
        )
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let token = format!(
        "Bearer {}",
        common::generate_delegated_test_token(&resources, &rider.user).await
    );

    let envelope = post(
        &app,
        "/a2a/jsonrpc",
        &token,
        &json!({
            "jsonrpc": "2.0",
            "method": "SendMessage",
            "params": {
                "message": {
                    "messageId": "m-labels-1",
                    "role": "ROLE_USER",
                    "parts": [{ "data": {
                        "tool_name": "get_activities",
                        "parameters": activities_from(WAHOO)
                    } }]
                }
            },
            "id": 1
        }),
    )
    .await;

    let task = &envelope["result"]["task"];
    assert_eq!(
        task["status"]["state"], "TASK_STATE_COMPLETED",
        "{envelope}"
    );
    let artifact = &task["artifacts"][0];
    assert!(
        artifact["parts"].to_string().contains("Riverside Loop"),
        "{envelope}"
    );
    assert_eq!(
        artifact["metadata"][TRUST_LABELS_META_KEY],
        device_sessions_label(),
        "{envelope}"
    );
}

// ============================================================================
// What the label says
// ============================================================================

#[tokio::test]
async fn a_load_history_is_t0() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;
    hold(&resources, &rider, WAHOO, &[session(WAHOO)]).await;

    let label = label(
        &resources,
        &rider,
        "get_training_history",
        json!({ "provider": WAHOO }),
    )
    .await
    .expect("the history served the athlete's sessions");
    assert_eq!(label["tier"], "T0", "{label}");
    assert_eq!(label["sensitive"], false, "{label}");
    assert_eq!(label["source_class"], json!(["device"]), "{label}");
}

#[tokio::test]
async fn a_session_picked_by_name_makes_the_intelligence_untrusted() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;

    // Asked about a session the athlete does not have, the tool picks one
    // and names it, and the sessions it could have picked.
    let response = call(
        &resources,
        &rider,
        Transport::McpHttp,
        true,
        "get_activity_intelligence",
        json!({ "provider": WAHOO, "activity_id": "no-such-session" }),
    )
    .await;
    assert!(response.success, "{:?}", response.error);
    assert!(
        response
            .result
            .as_ref()
            .is_some_and(|r| r.to_string().contains("Riverside Loop")),
        "the picked session is named: {:?}",
        response.result
    );
    let label = labels_of(&response).expect("the session was served");
    assert_eq!(label["untrusted"], true, "{label}");
}

#[tokio::test]
async fn a_coaching_platforms_item_is_another_apps_and_untrusted() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, INTERVALS_ICU).await;

    let label = label(
        &resources,
        &rider,
        "get_activities",
        activities_from(INTERVALS_ICU),
    )
    .await
    .expect("the platform's session was served");
    assert_eq!(label["source_class"], json!(["third_party_app"]), "{label}");
    assert_eq!(label["untrusted"], true, "{label}");
}

#[tokio::test]
async fn every_history_the_athlete_brought_in_is_the_devices_record() {
    let resources = server().await;
    for key in USER_PROVIDED_PROVIDERS {
        let rider = athlete(&resources).await;
        hold(&resources, &rider, key, &[session(key)]).await;

        let response = call(
            &resources,
            &rider,
            Transport::McpHttp,
            true,
            "get_activities",
            activities_from(key),
        )
        .await;
        assert!(response.success, "{key}: {:?}", response.error);
        assert!(
            response
                .result
                .as_ref()
                .is_some_and(|r| r.to_string().contains("Riverside Loop")),
            "{key}: the athlete's own copy reaches an external caller: {:?}",
            response.result
        );
        let label = labels_of(&response).unwrap_or_else(|| panic!("{key}: no label"));
        assert_eq!(label["source_class"], json!(["device"]), "{key}: {label}");
    }
}

/// The agent remembers `morning workouts` in Dravr's own app, approved so a
/// recall serves it.
async fn remember_approved(resources: &Arc<ServerContext>, rider: &Rider) {
    let remembered = call(
        resources,
        rider,
        Transport::WebApp,
        false,
        "remember_fact",
        json!({
            "kind": "preference",
            "predicate_code": "prefer",
            "object": "morning workouts",
            "confidence": 0.95,
        }),
    )
    .await;
    assert!(remembered.success, "{:?}", remembered.error);
    let fact = remembered.result.expect("the fact was filed");
    resources
        .agent
        .database
        .repositories()
        .memory
        .approve_user_fact(
            fact["fact_id"].as_str().expect("the fact names its id"),
            rider.tenant,
            &rider.user_id.to_string(),
        )
        .await
        .unwrap();
}

/// T3 never goes to an outside client, `sensitive_t3` given or not
/// (ChefFamille 2026-11-25): the agent's memory and the dossier carrying it
/// are T3, so an MCP or A2A caller is refused both, and no T3 label is ever
/// sent. Dravr's own agent still reads them.
#[tokio::test]
async fn the_agents_memory_and_the_dossier_never_reach_an_outside_caller() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    allow_outside_memory(
        resources.agent.database.repositories(),
        rider.tenant.as_uuid(),
        rider.user_id,
    )
    .await;
    remember_approved(&resources, &rider).await;

    for transport in [Transport::McpHttp, Transport::A2a] {
        for tool in ["recall_user_memory", "export_dossier"] {
            let response = call(&resources, &rider, transport, true, tool, json!({})).await;
            assert!(
                !response.success,
                "bug: {tool} over {transport:?} served T3 under sensitive_t3"
            );
            let result = response
                .result
                .as_ref()
                .expect("the refusal carries a result");
            assert_eq!(
                result["error_code"], "sensitive_tier_withheld",
                "{tool} over {transport:?}: {result}"
            );
            assert!(
                !result.to_string().contains("morning workouts"),
                "bug: the refusal leaked the memory: {result}"
            );
            assert_eq!(
                labels_of(&response),
                None,
                "bug: a T3 label reached an outside caller ({tool}, {transport:?})"
            );
        }
    }

    let own = call(
        &resources,
        &rider,
        Transport::WebApp,
        false,
        "recall_user_memory",
        json!({}),
    )
    .await;
    assert!(
        own.success
            && own
                .result
                .as_ref()
                .is_some_and(|r| r.to_string().contains("morning workouts")),
        "bug: Dravr's own agent keeps its memory: {:?}",
        own.result
    );
}

/// T3 never goes to an outside client on any surface, `sensitive_t3` given
/// (ChefFamille 2026-11-25): `/mcp` `tools/call`, `GET /mcp/tools`, A2A
/// `SendMessage` and the REST memory route each carry nothing of the agent's
/// memory, while the athlete's own session still reads it.
#[tokio::test]
async fn no_outside_surface_serves_t3_under_sensitive_t3() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    allow_outside_memory(
        resources.agent.database.repositories(),
        rider.tenant.as_uuid(),
        rider.user_id,
    )
    .await;
    remember_approved(&resources, &rider).await;
    resources
        .a2a
        .a2a_client_manager
        .register_client(
            pierre_a2a::ClientRegistrationRequest {
                name: format!("t3-agent-{}", rider.user_id),
                description: "an agent writing its own replies".into(),
                capabilities: vec!["fitness-data-analysis".into()],
                redirect_uris: vec![],
                contact_email: "agent@example.com".into(),
            },
            rider.user_id,
        )
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let delegated = format!(
        "Bearer {}",
        common::generate_delegated_test_token(&resources, &rider.user).await
    );
    let leaks = |body: &Value| body.to_string().contains("morning workouts");

    for tool in ["recall_user_memory", "export_dossier"] {
        let body = post(
            &app,
            "/mcp",
            &delegated,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": {} }
            }),
        )
        .await;
        assert_eq!(body["result"]["isError"], true, "bug: /mcp {tool}: {body}");
        assert!(!leaks(&body), "bug: /mcp {tool} leaked T3: {body}");

        let envelope = post(
            &app,
            "/a2a/jsonrpc",
            &delegated,
            &json!({
                "jsonrpc": "2.0",
                "method": "SendMessage",
                "params": {
                    "message": {
                        "messageId": format!("m-t3-{tool}"),
                        "role": "ROLE_USER",
                        "parts": [{ "data": { "tool_name": tool, "parameters": {} } }]
                    }
                },
                "id": 1
            }),
        )
        .await;
        assert!(!leaks(&envelope), "bug: A2A {tool} leaked T3: {envelope}");
    }

    let discovery = app
        .clone()
        .oneshot(
            Request::get("/mcp/tools")
                .header("authorization", &delegated)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(discovery.status(), StatusCode::OK, "GET /mcp/tools answers");
    let bytes = to_bytes(discovery.into_body(), usize::MAX).await.unwrap();
    let catalogue: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(
        !leaks(&catalogue),
        "bug: /mcp/tools carried T3: {catalogue}"
    );

    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;
    let rest = |authorization: String| {
        let app = app.clone();
        async move {
            let response = app
                .oneshot(
                    Request::get("/api/memory/facts")
                        .header("authorization", authorization)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            (status, String::from_utf8_lossy(&bytes).into_owned())
        }
    };
    let (status, outside) = rest(key).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: REST over a key: {outside}"
    );
    assert!(
        !outside.contains("morning workouts"),
        "bug: REST leaked T3: {outside}"
    );
    let (status, own) = rest(session).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert!(
        own.contains("morning workouts"),
        "bug: the athlete's own session reads their memory: {own}"
    );
}

// ============================================================================
// No label: nothing served, a failure, Dravr's own agent
// ============================================================================

#[tokio::test]
async fn a_call_that_served_nothing_carries_no_label() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;

    let reference = call(
        &resources,
        &rider,
        Transport::McpHttp,
        true,
        "list_yoga_poses",
        json!({}),
    )
    .await;
    assert!(reference.success, "{:?}", reference.error);
    assert_eq!(
        labels_of(&reference),
        None,
        "a reference lookup served none of the athlete's items"
    );
}

#[tokio::test]
async fn an_mcp_error_result_sends_no_meta() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;
    // The tenant turned the tool off: the call answers with a refusal.
    resources
        .mcp
        .tool_selection
        .set_tool_override(rider.tenant, "get_activities", false, rider.user_id, None)
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let token = format!(
        "Bearer {}",
        common::generate_delegated_test_token(&resources, &rider.user).await
    );

    let body = post(
        &app,
        "/mcp",
        &token,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "get_activities", "arguments": activities_from(WAHOO) }
        }),
    )
    .await;

    let result = &body["result"];
    assert_eq!(result["isError"], true, "{body}");
    assert!(result.get("_meta").is_none(), "{body}");
}

#[tokio::test]
async fn dravrs_own_agent_gets_no_label() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, WAHOO).await;

    // In-app chat, messaging, and a chat turn reached with an MCP token:
    // Dravr writes the reply itself, so nothing labels the result.
    for transport in [Transport::WebApp, Transport::Messaging, Transport::McpHttp] {
        let response = call(
            &resources,
            &rider,
            transport,
            false,
            "get_activities",
            activities_from(WAHOO),
        )
        .await;
        assert!(response.success, "{transport:?}: {:?}", response.error);
        assert!(
            response
                .result
                .as_ref()
                .is_some_and(|r| r.to_string().contains("Riverside Loop")),
            "{transport:?}: the session was served"
        );
        assert_eq!(labels_of(&response), None, "{transport:?}");
    }
}
