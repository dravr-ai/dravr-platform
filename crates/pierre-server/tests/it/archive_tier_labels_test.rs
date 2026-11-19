// ABOUTME: The trust label an MCP or A2A tool result carries in _meta: tier, source class, sensitive, untrusted (owl phase 1)
// ABOUTME: Drives /mcp, /a2a and the executor with stand-in providers; Dravr's own agent and a result serving nothing get none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{Duration, Utc};
use pierre_core::constants::oauth_providers::{INTERVALS_ICU, USER_PROVIDED_PROVIDERS, WAHOO};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ActivityBuilder, Athlete, ConnectionType, SportType, TenantId, User, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_mcp_server::context::ServerContext;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_tool_runtime::protocol::trust_labels::{labels_of, TRUST_LABELS_META_KEY};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user_with_plan};

/// A stand-in for a provider key, with terms that let its data travel over
/// every transport, so what it serves reaches an MCP or A2A caller. The key
/// is what sets the item's source class; the terms are the test's.
struct StandIn {
    name: &'static str,
}

impl ProviderDescriptor for StandIn {
    fn name(&self) -> &'static str {
        self.name
    }

    fn display_name(&self) -> &'static str {
        "Stand-in"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::ACTIVITIES
    }

    fn oauth_endpoints(&self) -> Option<OAuthEndpoints> {
        None
    }

    fn oauth_params(&self) -> Option<OAuthParams> {
        None
    }

    fn oauth_refresh(&self) -> Option<OAuthRefresh> {
        None
    }

    fn api_base_url(&self) -> &'static str {
        "http://localhost/stand-in"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    fn transport_policy(&self) -> TransportPolicy {
        TransportPolicy::AnyTransport
    }
}

/// One session the stand-in under `provider` answers with.
fn session(provider: &str) -> Activity {
    ActivityBuilder::new(
        format!("{provider}-run"),
        "Riverside Loop",
        SportType::Run,
        Utc::now() - Duration::days(1),
        3_600,
        provider,
    )
    .distance_meters(10_000.0)
    .average_heart_rate(148)
    .build()
}

/// The account behind a stand-in, answering with `activities`.
struct Account {
    config: ProviderConfig,
    provider: &'static str,
    activities: Vec<Activity>,
}

#[async_trait]
impl FitnessProvider for Account {
    fn name(&self) -> &'static str {
        self.provider
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    async fn set_credentials(&self, _credentials: OAuth2Credentials) -> AppResult<()> {
        Ok(())
    }

    async fn is_authenticated(&self) -> bool {
        true
    }

    async fn refresh_token_if_needed(&self) -> AppResult<()> {
        Ok(())
    }

    async fn get_athlete(&self) -> AppResult<Athlete> {
        Ok(Athlete {
            active_streak: None,
            created_at: None,
            id: format!("{}-athlete", self.provider),
            username: "stand_in".to_owned(),
            firstname: None,
            lastname: None,
            profile_picture: None,
            provider: self.provider.to_owned(),
            sex: None,
        })
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(self.activities.clone())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(self.activities.clone(), None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        self.activities
            .iter()
            .find(|a| a.id() == id)
            .cloned()
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }
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
        Some(Utc::now() + Duration::hours(6)),
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

#[tokio::test]
async fn the_agents_memory_is_t3_sensitive_and_agent_written() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    // The agent remembers in Dravr's own app; an MCP caller recalls it.
    let remembered = call(
        &resources,
        &rider,
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
    assert_eq!(fact["status"], "draft", "{fact}");
    assert_eq!(
        label(&resources, &rider, "recall_user_memory", json!({})).await,
        None,
        "a draft is not recalled, so nothing was served"
    );
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

    let label = label(&resources, &rider, "recall_user_memory", json!({}))
        .await
        .expect("the remembered fact was served");
    assert_eq!(
        label,
        json!({
            "v": 1,
            "tier": "T3",
            "source_class": ["agent_written"],
            "sensitive": true,
            "untrusted": true
        }),
        "the agent's memory is T3, sensitive and the agent's own unchecked writing"
    );
}

#[tokio::test]
async fn the_dossier_carrying_the_agents_memory_is_t3_and_agent_written() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    assert_eq!(
        label(&resources, &rider, "export_dossier", json!({})).await,
        None,
        "an empty dossier served nothing"
    );
    let remembered = call(
        &resources,
        &rider,
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

    let response = call(
        &resources,
        &rider,
        Transport::McpHttp,
        true,
        "export_dossier",
        json!({}),
    )
    .await;
    assert!(response.success, "{:?}", response.error);
    assert!(
        response
            .result
            .as_ref()
            .is_some_and(|r| r.to_string().contains("morning workouts")),
        "the remembered fact is in the dossier: {:?}",
        response.result
    );
    let label = labels_of(&response).expect("the dossier served the agent's memory");
    assert_eq!(label["tier"], "T3", "{label}");
    assert_eq!(label["sensitive"], true, "{label}");
    assert_eq!(label["source_class"], json!(["agent_written"]), "{label}");
    assert_eq!(label["untrusted"], true, "{label}");
}

#[tokio::test]
async fn the_dossier_labels_the_sport_anchors_it_carries() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    // The athlete states one per-sport anchor in Dravr's own app; the profile
    // beside it holds no other value.
    let stated = call(
        &resources,
        &rider,
        Transport::WebApp,
        false,
        "set_physiology",
        json!({ "sport_anchors": [{
            "sport": "ride", "anchor": "map", "value": 380, "kind": "measured",
            "source": "lab", "measured_on": "2026-11-02"
        }]}),
    )
    .await;
    assert!(stated.success, "{:?}", stated.error);

    let response = call(
        &resources,
        &rider,
        Transport::McpHttp,
        true,
        "export_dossier",
        json!({}),
    )
    .await;
    assert!(response.success, "{:?}", response.error);
    assert!(
        response
            .result
            .as_ref()
            .is_some_and(|r| r["sport_anchors"].as_array().is_some_and(|a| a.len() == 1)),
        "the stated anchor is in the dossier: {:?}",
        response.result
    );
    let label = labels_of(&response).expect("the dossier served the athlete's anchor");
    assert_eq!(
        label["source_class"],
        json!(["patient_entered"]),
        "an anchor the athlete stated is theirs: {label}"
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
