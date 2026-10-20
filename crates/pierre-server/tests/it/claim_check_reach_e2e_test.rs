// ABOUTME: How far the claim checker reaches over MCP and A2A — verify_claim judges the caller's own data (carnet#852)
// ABOUTME: A direct MCP or A2A result that served the athlete's data says Dravr did not check the reply built on it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use pierre_core::models::StatsCoverage;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{Duration, Utc};
use pierre_core::ai_policy::SourcePolicy;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    Activity, ActivityBuilder, Athlete, ConnectionType, SportType, Stats, TenantId,
    ThresholdProvenance, User, UserOAuthToken, UserPhysiologicalProfile,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_mcp_server::context::ServerContext;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::provider_terms::{NOLIO, NOLIO_TRANSPORT};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user_with_plan};

/// A provider whose terms let its data travel over every transport.
const OPEN: &str = "openrunner";

/// A coach-platform relay whose terms keep its data inside Dravr's own
/// surfaces (Nolio §6.9).
const RELAY: &str = "nolio";

/// The wire value of the note, pinned as a literal: MCP clients and A2A
/// agents parse it.
const NOTE: &str = "_reply_verification";

struct OpenDescriptor;

impl ProviderDescriptor for OpenDescriptor {
    fn name(&self) -> &'static str {
        OPEN
    }

    fn display_name(&self) -> &'static str {
        "Open Runner"
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
        "http://localhost/openrunner"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }
}

struct RelayDescriptor;

impl ProviderDescriptor for RelayDescriptor {
    fn name(&self) -> &'static str {
        RELAY
    }

    fn display_name(&self) -> &'static str {
        "Nolio"
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
        "http://localhost/nolio"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    fn ai_policy(&self) -> &'static SourcePolicy {
        &NOLIO
    }

    fn transport_policy(&self) -> TransportPolicy {
        NOLIO_TRANSPORT
    }
}

/// The open provider's long run, recorded yesterday.
fn long_run() -> Activity {
    ActivityBuilder::new(
        "o-long",
        "Riverside Loop",
        SportType::Run,
        Utc::now() - Duration::days(1),
        6_300,
        OPEN,
    )
    .distance_meters(21_100.0)
    .average_heart_rate(152)
    .build()
}

/// The relay's long ride, a Garmin recording, two days ago.
fn relay_ride() -> Activity {
    ActivityBuilder::new(
        "n-ride",
        "Col Climb",
        SportType::Ride,
        Utc::now() - Duration::days(2),
        9_000,
        RELAY,
    )
    .distance_meters(63_400.0)
    .source("garmin")
    .build()
}

/// The open provider's account, as its API answers.
struct OpenRunner {
    config: ProviderConfig,
}

#[async_trait]
impl FitnessProvider for OpenRunner {
    fn name(&self) -> &'static str {
        OPEN
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
            id: "open-athlete".to_owned(),
            username: "open_runner".to_owned(),
            firstname: Some("Open".to_owned()),
            lastname: Some("Runner".to_owned()),
            profile_picture: None,
            provider: OPEN.to_owned(),
        })
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(vec![long_run()])
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(vec![long_run()], None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        Some(long_run())
            .filter(|a| a.id() == id)
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Ok(Stats {
            total_activities: 1,
            total_distance: 21_100.0,
            total_duration: 6_300,
            total_elevation_gain: 0.0,
            coverage: StatsCoverage::Lifetime,
            year_to_date: None,
        })
    }
}

fn open_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(OpenRunner { config })
}

/// The relay is only ever read from the cache here: its factory is never
/// asked for a live client.
fn relay_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(OpenRunner { config })
}

/// The test server, with the two providers registered.
async fn server() -> Arc<ServerContext> {
    common::init_server_config();
    let base = create_test_server_resources().await.unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register_provider_bundle(ProviderBundle::new(Box::new(OpenDescriptor), open_factory));
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(RelayDescriptor),
        relay_factory,
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

/// A fresh athlete with nothing connected.
async fn athlete(resources: &ServerContext) -> Rider {
    let email = format!("reach-{}@example.com", Uuid::new_v4());
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

/// `verify_claim` on `claim`, about the athlete's own data.
async fn verify(
    resources: &Arc<ServerContext>,
    athlete: &Rider,
    transport: Transport,
    claim: &str,
) -> Value {
    let response = call(
        resources,
        athlete,
        transport,
        true,
        "verify_claim",
        json!({ "claim": claim, "category": "athlete_data" }),
    )
    .await;
    assert!(response.success, "{:?}", response.error);
    response.result.expect("a verdict")
}

fn activities_args() -> Value {
    json!({ "provider": OPEN, "limit": 10, "mode": "summary" })
}

// ============================================================================
// The note on a result whose caller writes its own reply
// ============================================================================

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
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn an_mcp_result_with_the_athletes_data_says_dravr_did_not_check_the_reply() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, OPEN).await;
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
            "params": { "name": "get_activities", "arguments": activities_args() }
        }),
    )
    .await;

    let structured = &body["result"]["structuredContent"];
    assert!(
        structured.to_string().contains("Riverside Loop"),
        "the athlete's data reached the client: {body}"
    );
    let note = &structured[NOTE];
    assert_eq!(note["reply_checked"], false, "{body}");
    assert_eq!(note["verify_with"], "verify_claim", "{body}");
}

#[tokio::test]
async fn an_a2a_task_with_the_athletes_data_says_dravr_did_not_check_the_reply() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, OPEN).await;
    resources
        .a2a
        .a2a_client_manager
        .register_client(
            pierre_a2a::ClientRegistrationRequest {
                name: format!("reach-agent-{}", rider.user_id),
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
                    "messageId": "m-reach-1",
                    "role": "ROLE_USER",
                    "parts": [{ "data": {
                        "tool_name": "get_activities",
                        "parameters": activities_args()
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
    // The athlete's figures leave as data; the task's own words are a fixed
    // status line, never prose about the athlete for a checker to read.
    assert_eq!(
        task["status"]["message"]["parts"][0]["text"], "Tool get_activities completed",
        "{envelope}"
    );
    let artifacts = task["artifacts"].to_string();
    assert!(artifacts.contains("Riverside Loop"), "{envelope}");
    assert!(
        artifacts.contains(&format!("\"{NOTE}\"")) && artifacts.contains("\"reply_checked\":false"),
        "the agent is told the reply it writes is unchecked: {envelope}"
    );
}

#[tokio::test]
async fn a_result_dravrs_own_agent_reads_carries_no_note() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, OPEN).await;

    // Dravr's own surfaces, and a chat turn reached with an MCP token: the
    // reply Dravr writes from these passes its verification stage.
    for transport in [Transport::WebApp, Transport::Messaging, Transport::McpHttp] {
        let response = call(
            &resources,
            &rider,
            transport,
            false,
            "get_activities",
            activities_args(),
        )
        .await;
        assert!(response.success, "{:?}", response.error);
        let result = response.result.unwrap();
        assert!(result.to_string().contains("Riverside Loop"), "{result}");
        assert!(result.get(NOTE).is_none(), "{transport:?}: {result}");
    }
}

#[tokio::test]
async fn a_result_built_on_a_nested_dispatch_still_says_the_reply_is_unchecked() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, OPEN).await;

    // `analyze_activity` reads nothing itself: it dispatches into
    // `get_activity_intelligence`, which fetches the run. The athlete's
    // figures reach the client all the same, so the note must too.
    let response = call(
        &resources,
        &rider,
        Transport::McpHttp,
        true,
        "analyze_activity",
        json!({ "provider": OPEN, "activity_id": "o-long" }),
    )
    .await;
    assert!(response.success, "{:?}", response.error);
    let result = response.result.unwrap();
    assert!(result.to_string().contains("21.1"), "{result}");
    assert_eq!(result[NOTE]["reply_checked"], false, "{result}");
    assert_eq!(result[NOTE]["verify_with"], "verify_claim", "{result}");
}

// ============================================================================
// verify_claim judges the caller's own data
// ============================================================================

#[tokio::test]
async fn verify_claim_contradicts_a_figure_about_an_athlete_with_nothing_connected() {
    let resources = server().await;
    let rider = athlete(&resources).await;

    let verdict = verify(
        &resources,
        &rider,
        Transport::McpHttp,
        "You covered 21 km on your last run.",
    )
    .await;

    // A figure about the athlete is judged against their own record, not the
    // literature: with nothing connected, no source could have produced it.
    assert_eq!(verdict["status"], "contradicted", "{verdict}");
    assert_eq!(verdict["layer_fired"], "athlete_data", "{verdict}");
    assert!(
        verdict["explanation"]
            .as_str()
            .unwrap()
            .contains("no connected data source"),
        "{verdict}"
    );
}

#[tokio::test]
async fn verify_claim_supports_a_figure_the_athlete_recorded() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, OPEN).await;
    hold(&resources, &rider, OPEN, &[long_run()]).await;

    for transport in [Transport::McpHttp, Transport::A2a] {
        let verdict = verify(
            &resources,
            &rider,
            transport,
            "You covered 21 km on your last run.",
        )
        .await;
        assert_eq!(verdict["status"], "supported", "{transport:?}: {verdict}");
        assert_eq!(verdict["layer_fired"], "athlete_data", "{verdict}");
        // The verifier read the athlete's activities to judge the claim and
        // served none of them: its own result carries no note.
        assert!(verdict.get(NOTE).is_none(), "{verdict}");
    }
}

#[tokio::test]
async fn verify_claim_never_rests_on_data_the_transport_withholds() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, RELAY).await;
    hold(&resources, &rider, RELAY, &[relay_ride()]).await;
    let claim = "Your ride two days ago covered 63 km.";

    // Over MCP and A2A the relay's ride is withheld before the layer sees
    // it, so it cannot confirm the figure, and nothing of it leaks into the
    // verdict.
    for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
        let verdict = verify(&resources, &rider, transport, claim).await;
        assert_ne!(verdict["status"], "supported", "{transport:?}: {verdict}");
        assert!(
            !verdict.to_string().contains("Col Climb"),
            "{transport:?}: {verdict}"
        );
    }

    // Dravr's own agent, calling the tool mid-turn on its own surface, judges
    // against the ride.
    let verdict = verify(&resources, &rider, Transport::WebApp, claim).await;
    assert_eq!(verdict["status"], "supported", "{verdict}");
    assert_eq!(verdict["layer_fired"], "athlete_data", "{verdict}");
}

fn profile(user_id: Uuid) -> UserPhysiologicalProfile {
    UserPhysiologicalProfile {
        user_id,
        vo2_max: Some(52.0),
        resting_hr: Some(48),
        max_hr: Some(190),
        lactate_threshold_percentage: Some(0.85),
        threshold_hr: None,
        age: Some(34),
        weight: Some(68.0),
        fitness_level: FitnessLevel::Intermediate,
        primary_sport: SportType::Ride,
        training_experience_years: Some(6),
        ftp_watts: Some(250),
        threshold_pace_sec_per_km: None,
        hr_zones: None,
        power_zones: None,
        critical_power_watts: None,
        w_prime_joules: None,
        critical_speed_mps: None,
        d_prime_meters: None,
        threshold_provenance: ThresholdProvenance::default(),
        transport_policy: TransportPolicy::AnyTransport,
    }
}

/// Sixteen distinct training days, clearing the personalized layer's
/// stale-snapshot gate.
fn training_days() -> Vec<Activity> {
    (1..=16)
        .map(|days_ago| {
            ActivityBuilder::new(
                format!("o-day-{days_ago}"),
                format!("Endurance {days_ago}"),
                SportType::Ride,
                Utc::now() - Duration::days(days_ago),
                3_600,
                OPEN,
            )
            .distance_meters(30_000.0)
            .average_power(190)
            .normalized_power(200)
            .average_heart_rate(140)
            .build()
        })
        .collect()
}

#[tokio::test]
async fn verify_claim_judges_a_figure_against_the_athletes_own_physiology() {
    let resources = server().await;
    let rider = athlete(&resources).await;
    connect(&resources, &rider, OPEN).await;
    hold(&resources, &rider, OPEN, &training_days()).await;
    resources
        .agent
        .database
        .repositories()
        .user_physiological_profile
        .upsert_user_physiological_profile(rider.tenant, rider.user_id, &profile(rider.user_id))
        .await
        .unwrap();

    let invented = verify(&resources, &rider, Transport::McpHttp, "Your FTP is 330 W.").await;
    assert_eq!(invented["status"], "contradicted", "{invented}");
    assert_eq!(invented["layer_fired"], "personalized", "{invented}");

    let recorded = verify(&resources, &rider, Transport::McpHttp, "Your FTP is 250 W.").await;
    assert_eq!(recorded["status"], "supported", "{recorded}");
    assert_eq!(recorded["layer_fired"], "personalized", "{recorded}");
}
