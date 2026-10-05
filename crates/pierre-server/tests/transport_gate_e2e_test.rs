// ABOUTME: End-to-end check of the per-provider transport gate through the real executor and HTTP app (carnet#724)
// ABOUTME: A first-party-only relay, and content stamped as derived from it (carnet#769), stay on Dravr's own surfaces

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use chrono::{Duration, DurationRound, Utc};
use embacle_tool_host::ToolSurface;
use pierre_auth::api_keys::{ApiKey, ApiKeyManager, ApiKeyTier};
use pierre_chat_pipeline::stages::persistence::get_conversation_history;
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_core::ai_policy::SourcePolicy;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    NewGroupTranscriptEntry, TranscriptSpeaker,
};
use pierre_core::models::{
    Activity, ActivityBuilder, AddMessageParams, Athlete, ConnectionType, ConversationTurnId,
    SportType, Stats, TenantId, User, UserOAuthToken, UserPhysiologicalProfile,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, SavePlanBundleParams, UpsertUserFactParams};
use pierre_mcp_server::context::ServerContext;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_providers::ai_scope;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::provider_terms::{NOLIO, NOLIO_TRANSPORT};
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user_with_plan};

const RELAY: &str = "nolio";

/// The key of the note a caller over an external transport reads. Pinned as a
/// literal on purpose: it is a wire value MCP, A2A and API-key clients parse.
const UNAVAILABLE_HERE: &str = "_unavailable_over_this_interface";

/// A coach-platform relay whose terms keep its data inside Dravr's own
/// surfaces (Nolio §6.9).
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

/// The relay's account: two Garmin-recorded sessions, which the Annex lets a
/// model see whole, so only the transport gate can hold them back.
struct Relay {
    config: ProviderConfig,
}

fn session(id: &str, name: &str, days_ago: i64) -> Activity {
    ActivityBuilder::new(
        id,
        name,
        SportType::Ride,
        Utc::now() - Duration::days(days_ago),
        3_600,
        RELAY,
    )
    .average_heart_rate(150)
    .source("garmin")
    .build()
}

fn sessions() -> Vec<Activity> {
    vec![
        session("g1", "Garmin Hills", 1),
        session("g2", "Garmin Tempo", 2),
    ]
}

#[async_trait]
impl FitnessProvider for Relay {
    fn name(&self) -> &'static str {
        RELAY
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
            id: "relay-athlete".to_owned(),
            username: "relay_rider".to_owned(),
            firstname: Some("Relay".to_owned()),
            lastname: Some("Rider".to_owned()),
            profile_picture: None,
            provider: RELAY.to_owned(),
        })
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(sessions())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(sessions(), None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        sessions()
            .into_iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Ok(Stats {
            total_activities: 412,
            total_distance: 9_876_000.0,
            total_duration: 1_234_567,
            total_elevation_gain: 54_321.0,
            year_to_date: None,
        })
    }
}

fn relay_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(Relay { config })
}

/// The test server, with the relay registered beside the built-in providers.
async fn server_with_relay() -> Arc<ServerContext> {
    let base = create_test_server_resources().await.unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(RelayDescriptor),
        relay_factory,
    ));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

/// An athlete with the relay connected.
struct Rider {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
}

async fn athlete_with_relay(resources: &ServerContext) -> Rider {
    let email = format!("relay-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let repos = resources.agent.database.repositories();
    let token = UserOAuthToken::new(
        user_id,
        tenant.to_string(),
        RELAY.to_owned(),
        "relay-access".to_owned(),
        Some("relay-refresh".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(user_id, tenant, RELAY, &ConnectionType::OAuth, None)
        .await
        .unwrap();
    Rider {
        user,
        user_id,
        tenant,
    }
}

/// Run `tool` for the athlete on an executor bound to `transport` (`None`:
/// an entry point that declared nothing).
async fn call(
    resources: &Arc<ServerContext>,
    athlete: &Rider,
    transport: Option<Transport>,
    tool: &str,
) -> UniversalResponse {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalExecutor::new(runtime).with_scopes(OAuthScope::self_grant());
    let executor = match transport {
        Some(served) => executor.with_transport(served),
        None => executor,
    };
    executor
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters: serde_json::json!({ "provider": RELAY, "limit": 10, "mode": "summary" }),
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool answers")
}

fn payload_text(response: &UniversalResponse) -> String {
    response
        .result
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default()
}

#[tokio::test]
async fn the_same_call_returns_the_relays_items_only_to_dravrs_own_surfaces() {
    let resources = server_with_relay().await;
    let athlete = athlete_with_relay(&resources).await;

    // External first: nothing it writes may starve the first-party call after.
    let external = call(
        &resources,
        &athlete,
        Some(Transport::McpHttp),
        "get_activities",
    )
    .await;
    assert!(external.success, "{:?}", external.error);
    let text = payload_text(&external);
    assert!(!text.contains("Garmin Hills"), "{text}");
    assert!(!text.contains("Garmin Tempo"), "{text}");
    let note = &external.result.as_ref().expect("a payload")[UNAVAILABLE_HERE];
    assert!(
        note["items_unavailable"].as_u64().unwrap_or(0) >= 2,
        "the caller is told data exists it cannot see: {note}"
    );
    assert!(
        !note.to_string().contains(RELAY),
        "the note never names the service"
    );

    let first_party = call(
        &resources,
        &athlete,
        Some(Transport::WebApp),
        "get_activities",
    )
    .await;
    assert!(first_party.success, "{:?}", first_party.error);
    let text = payload_text(&first_party);
    assert!(text.contains("Garmin Hills"), "{text}");
    assert!(text.contains("Garmin Tempo"), "{text}");
    assert!(!text.contains(UNAVAILABLE_HERE), "{text}");

    // And external again, after a first-party call warmed every cache.
    for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
        let replay = call(&resources, &athlete, Some(transport), "get_activities").await;
        let text = payload_text(&replay);
        assert!(
            !text.contains("Garmin Hills"),
            "{transport:?} must not replay a first-party list: {text}"
        );
    }

    // The athlete's own cache, written through by those calls, keeps every row.
    let cached = resources
        .agent
        .database
        .repositories()
        .activity_cache
        .get_cached_activities(
            athlete.user_id,
            &athlete.tenant,
            Some(RELAY),
            Utc::now() - Duration::days(30),
            Utc::now(),
            50,
        )
        .await
        .expect("cache read");
    let mut names: Vec<&str> = cached.iter().map(Activity::name).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Garmin Hills", "Garmin Tempo"]);
}

#[tokio::test]
async fn an_entry_point_that_declares_nothing_is_served_as_external() {
    let resources = server_with_relay().await;
    let athlete = athlete_with_relay(&resources).await;

    let undeclared = call(&resources, &athlete, None, "get_activities").await;
    let text = payload_text(&undeclared);
    assert!(!text.contains("Garmin Hills"), "{text}");
    assert!(text.contains(UNAVAILABLE_HERE), "{text}");
}

#[tokio::test]
async fn an_executor_built_inside_an_external_call_stays_external() {
    let resources = server_with_relay().await;
    let athlete = athlete_with_relay(&resources).await;

    // A nested dispatch declaring first-party inside an MCP call: the
    // declaration only narrows, so it is served as MCP.
    let nested = ai_scope::serve_over(
        Transport::McpHttp,
        call(
            &resources,
            &athlete,
            Some(Transport::WebApp),
            "get_activities",
        ),
    )
    .await;
    let text = payload_text(&nested);
    assert!(!text.contains("Garmin Hills"), "{text}");
}

#[tokio::test]
async fn profile_and_stats_are_refused_externally_even_from_a_warm_cache() {
    let resources = server_with_relay().await;
    let athlete = athlete_with_relay(&resources).await;

    // First-party reads reach the relay and warm the profile and stats caches.
    let profile = call(
        &resources,
        &athlete,
        Some(Transport::MobileApp),
        "get_athlete",
    )
    .await;
    assert!(profile.success, "{:?}", profile.error);
    assert!(payload_text(&profile).contains("relay_rider"));
    let stats = call(
        &resources,
        &athlete,
        Some(Transport::MobileApp),
        "get_stats",
    )
    .await;
    assert!(stats.success, "{:?}", stats.error);
    assert!(
        payload_text(&stats).contains("412"),
        "{}",
        payload_text(&stats)
    );

    for tool in ["get_athlete", "get_stats"] {
        let refused = call(&resources, &athlete, Some(Transport::McpHttp), tool).await;
        assert!(!refused.success, "{tool} over MCP");
        let text = payload_text(&refused);
        assert!(!text.contains("relay_rider"), "{tool}: {text}");
        assert!(!text.contains("412"), "{tool}: {text}");
        assert!(
            refused
                .error
                .as_deref()
                .is_some_and(|message| message.contains("not available over this interface")),
            "{tool}: {:?}",
            refused.error
        );
        assert_eq!(
            refused.result.as_ref().expect("a payload")[UNAVAILABLE_HERE]["items_unavailable"],
            1,
            "{tool}: a neutral note, never an auth or reconnect signal"
        );
    }
}

// ── The REST routes, through the production HTTP app ─────────────────────

/// A stored API key for `user_id`, and the full key to present.
async fn api_key(resources: &Arc<ServerContext>, user_id: Uuid) -> String {
    let data = ApiKeyManager::new().generate_api_key(false);
    let key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "transport gate key".to_owned(),
        key_prefix: data.key_prefix,
        key_hash: data.key_hash,
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 1_000,
        rate_limit_window_seconds: 3_600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    resources.common.repos.api_keys.create(&key).await.unwrap();
    data.full_key
}

async fn get(app: &Router, uri: &str, authorization: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::get(uri)
                .header("authorization", authorization)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// The cached copies the Home routes read: the relay's recording of a ride and
/// another service's recording of the same ride, plus a ride only the relay
/// holds. Only the relay's copy of the shared ride carries a distance, so the
/// merged workout shows one only when the relay's copy was merged in.
async fn seed_home(resources: &Arc<ServerContext>, athlete: &Rider) {
    let start = (Utc::now() - Duration::days(1))
        .duration_trunc(Duration::minutes(1))
        .unwrap();
    let relay_copy = ActivityBuilder::new(
        "n-shared",
        "Relay Long Ride",
        SportType::Ride,
        start,
        7_200,
        RELAY,
    )
    .distance_meters(42_000.0)
    .source("garmin")
    .build();
    let relay_only = ActivityBuilder::new(
        "n-only",
        "Relay Recovery Spin",
        SportType::Ride,
        start - Duration::days(1),
        2_400,
        RELAY,
    )
    .source("garmin")
    .build();
    let other_copy = ActivityBuilder::new(
        "o-shared",
        "Open Long Ride",
        SportType::Ride,
        start,
        7_200,
        "openprovider",
    )
    .build();
    let cache = &resources.common.repos.activity_cache;
    cache
        .upsert_activities(
            athlete.user_id,
            &athlete.tenant,
            RELAY,
            &[relay_copy, relay_only],
        )
        .await
        .unwrap();
    cache
        .upsert_activities(
            athlete.user_id,
            &athlete.tenant,
            "openprovider",
            &[other_copy],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn an_api_key_reads_home_without_the_relay_and_a_session_reads_all_of_it() {
    let resources = server_with_relay().await;
    let athlete = athlete_with_relay(&resources).await;
    seed_home(&resources, &athlete).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &athlete.user).await
    );
    let key = api_key(&resources, athlete.user_id).await;

    let (status, own_app) = get(&app, "/api/me/activities/recent?limit=10", &session).await;
    assert_eq!(status, StatusCode::OK, "{own_app}");
    let text = own_app.to_string();
    assert!(text.contains("Relay Recovery Spin"), "{text}");
    assert!(
        text.contains("42000"),
        "the athlete's app merges the relay's distance into the shared ride: {text}"
    );

    let (status, external) = get(&app, "/api/me/activities/recent?limit=10", &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let text = external.to_string();
    assert!(
        !text.contains("Relay"),
        "no relay row over an API key: {text}"
    );
    assert!(
        !text.contains("42000"),
        "the relay's copy is dropped before the merge, so its distance never rides on the other copy: {text}"
    );
    assert!(text.contains("Open Long Ride"), "{text}");

    let detail = format!("/api/me/activities/{RELAY}/n-only");
    let (status, refused) = get(&app, &detail, &key).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(
        !refused.to_string().contains("Relay Recovery Spin"),
        "{refused}"
    );
    let (status, shown) = get(&app, &detail, &session).await;
    assert_eq!(status, StatusCode::OK, "{shown}");
    assert!(shown.to_string().contains("Relay Recovery Spin"), "{shown}");
}

/// One `get_activities` call for the relay's rows over `POST /mcp`.
fn mcp_relay_call(authorization: &str) -> Request<Body> {
    Request::post("/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", authorization)
        .body(Body::from(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "get_activities",
                    "arguments": { "provider": RELAY, "limit": 10, "mode": "summary" }
                }
            })
            .to_string(),
        ))
        .unwrap()
}

#[tokio::test]
async fn mcp_over_http_is_external_whatever_credential_reaches_it() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);

    // The athlete's own session token, pasted into an MCP client, is not an
    // MCP credential at all: refused before any tool runs (carnet#768).
    let session = common::generate_test_token(&resources, &rider.user).await;
    let refused = app
        .clone()
        .oneshot(mcp_relay_call(&format!("Bearer {session}")))
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

    // Every credential `/mcp` does take is served as external: the route
    // decides, never the token. The athlete's API key carries the self grant
    // and a delegated grant does not; neither reads the relay's rows.
    let key = format!("Bearer {}", api_key(&resources, rider.user_id).await);
    let delegated = format!(
        "Bearer {}",
        common::generate_delegated_test_token(&resources, &rider.user).await
    );
    for authorization in [key.as_str(), delegated.as_str()] {
        let response = app
            .clone()
            .oneshot(mcp_relay_call(authorization))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8_lossy(&bytes);
        assert!(body.contains("\"result\""), "{body}");
        assert!(!body.contains("Garmin Hills"), "{body}");
        assert!(body.contains(UNAVAILABLE_HERE), "{body}");
    }
}

/// The Copilot tool loop: its surface is built in the turn's task and called
/// from the loopback listener's task, where no task-local survives.
fn copilot_surface(resources: &Arc<ServerContext>, rider: &Rider) -> TurnToolSurface {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    HostedToolBridge::new(
        true,
        resources.mcp.tool_registry.clone(),
        resources.common.repos.clone(),
        tool_runtime,
    )
    .turn_surface(ToolSessionTurn {
        user_id: &rider.user_id.to_string(),
        tenant_id: rider.tenant,
        conversation_id: "transport-gate-turn",
        turn_id: ConversationTurnId(Uuid::new_v4()),
        turn_agent_id: None,
        budget: 64,
        seat: TurnSeat::Subject,
    })
}

#[tokio::test]
async fn the_copilot_loop_serves_its_turns_transport_from_another_task() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let args = serde_json::json!({ "provider": RELAY, "limit": 10, "mode": "summary" });

    for (transport, sees_relay) in [(Transport::WebApp, true), (Transport::ApiKey, false)] {
        let surface =
            ai_scope::serve_over(transport, async { copilot_surface(&resources, &rider) }).await;
        let call_args = args.clone();
        let outcome = tokio::spawn(async move { surface.call("get_activities", &call_args).await })
            .await
            .unwrap();
        assert_eq!(
            outcome.text.contains("Garmin Hills"),
            sees_relay,
            "{transport:?}: {}",
            outcome.text
        );
    }
}

#[tokio::test]
async fn an_api_key_cannot_be_exchanged_for_a_session() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let key = api_key(&resources, rider.user_id).await;
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );

    let (status, refused) = get(&app, "/api/auth/session", &key).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["code"], "PermissionDenied", "{refused}");
    assert!(
        !refused.to_string().contains("eyJ"),
        "no token in the refusal: {refused}"
    );

    let switch = |authorization: &str| {
        Request::post("/tenants/switch")
            .header("content-type", "application/json")
            .header("authorization", authorization)
            .body(Body::from(
                serde_json::json!({ "tenant_id": rider.tenant.to_string() }).to_string(),
            ))
            .unwrap()
    };
    let response = app.clone().oneshot(switch(&key)).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let (status, restored) = get(&app, "/api/auth/session", &session).await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    let response = app.clone().oneshot(switch(&session)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn derived_training_status_leaves_the_relay_out_of_an_api_key_answer() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    // Real physiology, so each ride's load is HR-derived and non-zero.
    resources
        .common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(
            rider.tenant,
            rider.user_id,
            &UserPhysiologicalProfile {
                user_id: rider.user_id,
                vo2_max: Some(52.0),
                resting_hr: Some(50),
                max_hr: Some(190),
                lactate_threshold_percentage: Some(0.85),
                threshold_hr: None,
                age: Some(34),
                weight: Some(72.0),
                fitness_level: FitnessLevel::Advanced,
                primary_sport: SportType::Ride,
                training_experience_years: Some(10),
                ftp_watts: Some(280),
                threshold_pace_sec_per_km: Some(225.0),
                hr_zones: None,
                power_zones: None,
                critical_power_watts: None,
                w_prime_joules: None,
                critical_speed_mps: None,
                d_prime_meters: None,
                transport_policy: TransportPolicy::AnyTransport,
            },
        )
        .await
        .unwrap();
    // Two hundred days of the relay's rides, one a day — deep enough for a
    // reading — and the only data the athlete has.
    let history: Vec<Activity> = (0..=200)
        .map(|day| {
            ActivityBuilder::new(
                format!("n-day-{day}"),
                format!("Relay Day {day}"),
                SportType::Ride,
                Utc::now() - Duration::days(day),
                3_600,
                RELAY,
            )
            .distance_meters(30_000.0)
            .average_heart_rate(150)
            .source("garmin")
            .build()
        })
        .collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(rider.user_id, &rider.tenant, RELAY, &history)
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;

    let (status, own_app) = get(&app, "/api/me/training-status", &session).await;
    assert_eq!(status, StatusCode::OK, "{own_app}");
    let own_trend = own_app["trend"].as_array().map_or(0, Vec::len);
    assert!(
        own_trend > 0,
        "the athlete's app computes from the relay's history: {own_app}"
    );

    let (status, external) = get(&app, "/api/me/training-status", &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    assert_eq!(
        external["trend"].as_array().map_or(0, Vec::len),
        0,
        "over an API key the status is computed without the relay's rows: {external}"
    );
    assert!(external["form"].is_null(), "{external}");
}

// ── Derived content: transcripts, facts, plans, a chat turn ──────────────

async fn send_json(
    app: &Router,
    method: &str,
    uri: &str,
    authorization: &str,
    body: &Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .header("authorization", authorization)
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Write one chat row straight into the thread, stamped as the turn that
/// wrote it would have stamped it.
async fn write_row(
    resources: &Arc<ServerContext>,
    rider: &Rider,
    conversation_id: &str,
    role: &str,
    content: &str,
    transport_policy: TransportPolicy,
) {
    resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: rider.tenant,
            conversation_id,
            user_id: &rider.user_id.to_string(),
            role,
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy,
        })
        .await
        .unwrap();
}

/// A fact the athlete holds, stamped as given.
async fn write_fact(
    resources: &Arc<ServerContext>,
    rider: &Rider,
    object: &str,
    transport_policy: TransportPolicy,
) {
    resources
        .common
        .repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: rider.tenant,
            user_id: &rider.user_id.to_string(),
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: None,
            predicate_code: PredicateCode::Prefer,
            object,
            confidence: 0.9,
            source: FactSource::Conversation,
            valid_until: None,
            source_msg_id: None,
            transport_policy,
        })
        .await
        .unwrap();
}

/// A conversation the athlete opened in the app.
async fn open_thread(app: &Router, session: &str, title: &str) -> String {
    let (status, created) = send_json(
        app,
        "POST",
        "/api/chat/conversations",
        session,
        &serde_json::json!({ "title": title }),
    )
    .await;
    assert!(status.is_success(), "{created}");
    created["id"]
        .as_str()
        .expect("a conversation id")
        .to_owned()
}

#[tokio::test]
async fn a_stamped_message_is_withheld_and_an_unstamped_one_served_over_an_api_key() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;

    let conversation = open_thread(&app, &session, "Week review").await;
    write_row(
        &resources,
        &rider,
        &conversation,
        "user",
        "How was my week?",
        TransportPolicy::AnyTransport,
    )
    .await;
    write_row(
        &resources,
        &rider,
        &conversation,
        "assistant",
        "Open coaching: keep Sunday easy.",
        TransportPolicy::AnyTransport,
    )
    .await;
    write_row(
        &resources,
        &rider,
        &conversation,
        "assistant",
        "Relay-derived: Garmin Hills was your strongest climb.",
        TransportPolicy::FirstPartyOnly,
    )
    .await;
    let messages = format!("/api/chat/conversations/{conversation}/messages");

    // The athlete holds the relay, yet the API key reads the thread: only the
    // row derived from the relay is withheld.
    let (status, external) = get(&app, &messages, &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let text = external.to_string();
    assert!(text.contains("How was my week?"), "{text}");
    assert!(text.contains("Open coaching"), "{text}");
    assert!(!text.contains("Garmin Hills"), "{text}");

    let (status, own) = get(&app, &messages, &session).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    let text = own.to_string();
    assert!(text.contains("Open coaching"), "{text}");
    assert!(
        text.contains("Garmin Hills"),
        "the athlete's own app reads every row: {text}"
    );

    // The list row previews the newest row the reader may read: the key sees
    // the open reply and counts two rows, the app the stamped one and three.
    let (status, external_list) = get(&app, "/api/chat/conversations", &key).await;
    assert_eq!(status, StatusCode::OK, "{external_list}");
    let row = external_list["conversations"]
        .as_array()
        .and_then(|rows| rows.iter().find(|c| c["id"] == conversation.as_str()))
        .cloned()
        .expect("the thread is listed over the key");
    assert_eq!(
        row["last_message"]["preview"], "Open coaching: keep Sunday easy.",
        "{row}"
    );
    assert_eq!(row["message_count"], 2, "{row}");
    let (_, own_list) = get(&app, "/api/chat/conversations", &session).await;
    assert!(own_list.to_string().contains("Relay-derived"), "{own_list}");

    // The prompt a turn assembles over the key replays the same rows.
    let repos = &resources.common.repos;
    let owner = rider.user_id.to_string();
    let replay = |transport| {
        ai_scope::serve_over(
            transport,
            ai_scope::derived(get_conversation_history(
                repos.chat.as_ref(),
                &conversation,
                &owner,
                rider.tenant,
                50,
            )),
        )
    };
    let (history, stamp) = replay(Transport::ApiKey).await;
    let contents: Vec<String> = history.unwrap().into_iter().map(|m| m.content).collect();
    assert_eq!(
        contents,
        vec![
            "How was my week?".to_owned(),
            "Open coaching: keep Sunday easy.".to_owned()
        ]
    );
    assert_eq!(stamp, TransportPolicy::AnyTransport);
    let (history, stamp) = replay(Transport::WebApp).await;
    assert_eq!(history.unwrap().len(), 3);
    assert_eq!(
        stamp,
        TransportPolicy::FirstPartyOnly,
        "a first-party turn replaying the stamped row is stamped by it"
    );
}

#[tokio::test]
async fn facts_and_the_plan_derived_from_the_relay_are_withheld_and_the_rest_served() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;

    write_fact(
        &resources,
        &rider,
        "morning runs",
        TransportPolicy::AnyTransport,
    )
    .await;
    write_fact(
        &resources,
        &rider,
        "the Garmin Hills loop",
        TransportPolicy::FirstPartyOnly,
    )
    .await;

    let (status, external) = get(&app, "/api/memory/facts", &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let text = external.to_string();
    assert!(text.contains("morning runs"), "{text}");
    assert!(!text.contains("Garmin Hills"), "{text}");
    let (_, own) = get(&app, "/api/memory/facts", &session).await;
    assert!(own.to_string().contains("Garmin Hills"), "{own}");

    for (transport, sees_relay) in [(Transport::McpHttp, false), (Transport::WebApp, true)] {
        let recall = call(&resources, &rider, Some(transport), "recall_user_memory").await;
        assert!(recall.success, "{transport:?}: {:?}", recall.error);
        let text = payload_text(&recall);
        assert!(text.contains("morning runs"), "{transport:?}: {text}");
        assert_eq!(
            text.contains("Garmin Hills"),
            sees_relay,
            "{transport:?}: {text}"
        );
    }

    // No plan yet: the key is told so, like the app.
    let (status, _) = get(&app, "/api/me/training-plan", &key).await;
    assert_eq!(status, StatusCode::OK);

    let goal = GoalRace {
        name: "Relay Gran Fondo".to_owned(),
        date: (Utc::now() + Duration::days(90))
            .date_naive()
            .format("%Y-%m-%d")
            .to_string(),
        discipline: "road".to_owned(),
        priority: RacePriority::A,
    };
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &rider.tenant.to_string(),
            user_id: &rider.user_id.to_string(),
            author: PlanAuthor::none(),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: Some(&[]),
                strategy: "built on the relay's rides",
                phases: &[],
                source_conversation_id: None,
                flavour: None,
                season_start: None,
                season_end: None,
            }),
            weeks: &[],
            transport_policy: TransportPolicy::FirstPartyOnly,
        })
        .await
        .unwrap();

    let (status, refused) = get(&app, "/api/me/training-plan", &key).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["code"], "UnavailableOverTransport", "{refused}");
    assert!(!refused.to_string().contains("Gran Fondo"), "{refused}");
    let (status, own) = get(&app, "/api/me/training-plan", &session).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert!(own.to_string().contains("Relay Gran Fondo"), "{own}");

    let plan_tool = call(
        &resources,
        &rider,
        Some(Transport::McpHttp),
        "get_training_plan",
    )
    .await;
    assert!(!payload_text(&plan_tool).contains("Gran Fondo"));
    assert!(payload_text(&plan_tool).contains(UNAVAILABLE_HERE));
}

#[tokio::test]
async fn a_turn_that_served_the_relay_stamps_what_its_tools_write() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let remember = |object: &str| {
        serde_json::json!({
            "kind": "preference",
            "predicate_code": "prefer",
            "object": object,
            "confidence": 0.9,
        })
    };

    // The Copilot loop calls from another task: the turn's accumulator rides
    // the surface there. A first-party turn that read the relay stamps the
    // fact it then writes; a turn over the key never read it, so its fact
    // stays open.
    for (transport, object, expected) in [
        (
            Transport::WebApp,
            "long climbs after the relay rides",
            TransportPolicy::FirstPartyOnly,
        ),
        (
            Transport::ApiKey,
            "short intervals",
            TransportPolicy::AnyTransport,
        ),
    ] {
        let provenance = ai_scope::Provenance::new();
        let surface = ai_scope::serve_over(
            transport,
            ai_scope::tracking(provenance.clone(), async {
                copilot_surface(&resources, &rider)
            }),
        )
        .await;
        let args = remember(object);
        let reads = serde_json::json!({ "provider": RELAY, "limit": 10, "mode": "summary" });
        tokio::spawn(async move {
            surface.call("get_activities", &reads).await;
            surface.call("remember_fact", &args).await
        })
        .await
        .unwrap();
        assert_eq!(provenance.policy(), expected, "{transport:?}");

        let facts = resources
            .common
            .repos
            .memory
            .list_user_facts(
                rider.tenant,
                &rider.user_id.to_string(),
                None,
                None,
                50,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .unwrap();
        let fact = facts
            .iter()
            .find(|f| f.object == object)
            .expect("the tool wrote the fact");
        assert_eq!(fact.transport_policy, expected, "{transport:?}");
    }

    let recall = call(
        &resources,
        &rider,
        Some(Transport::McpHttp),
        "recall_user_memory",
    )
    .await;
    let text = payload_text(&recall);
    assert!(text.contains("short intervals"), "{text}");
    assert!(!text.contains("long climbs"), "{text}");
}

#[tokio::test]
async fn a_thread_opened_from_a_relay_activity_is_withheld_whole_over_an_api_key() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;

    let thread = open_thread(&app, &session, "Garmin Hills").await;
    // The athlete's own question names the relay's activity: no row is
    // stamped, the link to the activity is what holds the thread.
    write_row(
        &resources,
        &rider,
        &thread,
        "user",
        "How did Garmin Hills go?",
        TransportPolicy::AnyTransport,
    )
    .await;
    // The app links the thread to the relay's activity it was opened from;
    // the link carries the activity's terms.
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(rider.user_id, &rider.tenant, RELAY, &sessions())
        .await
        .unwrap();
    let (status, linked) = send_json(
        &app,
        "PUT",
        &format!("/api/me/activities/{RELAY}/g1/conversation"),
        &session,
        &serde_json::json!({ "conversation_id": thread }),
    )
    .await;
    assert!(status.is_success(), "{linked}");
    let open = open_thread(&app, &session, "Open thread").await;

    let messages = format!("/api/chat/conversations/{thread}/messages");
    let (status, refused) = get(&app, &messages, &key).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["code"], "UnavailableOverTransport", "{refused}");
    let (status, own) = get(&app, &messages, &session).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert!(own.to_string().contains("Garmin Hills"), "{own}");

    let (_, external_list) = get(&app, "/api/chat/conversations", &key).await;
    let ids: Vec<&str> = external_list["conversations"]
        .as_array()
        .map(|rows| rows.iter().filter_map(|c| c["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(ids.contains(&open.as_str()), "{external_list}");
    assert!(!ids.contains(&thread.as_str()), "{external_list}");
    assert!(
        !external_list.to_string().contains("Garmin Hills"),
        "{external_list}"
    );
    let (_, own_list) = get(&app, "/api/chat/conversations", &session).await;
    assert!(own_list.to_string().contains(thread.as_str()), "{own_list}");
}

#[tokio::test]
async fn stamped_rows_filling_the_first_page_still_leave_an_external_page_full() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;

    // Three open facts, then five stamped ones written after them: newest
    // first, the stamped rows are the whole first page of three.
    for n in 0..3 {
        write_fact(
            &resources,
            &rider,
            &format!("open fact {n}"),
            TransportPolicy::AnyTransport,
        )
        .await;
        sleep(StdDuration::from_millis(5)).await;
    }
    for n in 0..5 {
        write_fact(
            &resources,
            &rider,
            &format!("relay fact {n}"),
            TransportPolicy::FirstPartyOnly,
        )
        .await;
        sleep(StdDuration::from_millis(5)).await;
    }
    let (status, external) = get(&app, "/api/memory/facts?limit=3", &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let objects: Vec<&str> = external["facts"]
        .as_array()
        .map(|facts| facts.iter().filter_map(|f| f["object"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(
        objects.len(),
        3,
        "the key's page holds `limit` readable facts: {external}"
    );
    assert!(
        objects.iter().all(|o| o.starts_with("open fact")),
        "{external}"
    );
    let (_, own) = get(&app, "/api/memory/facts?limit=3", &session).await;
    assert!(own.to_string().contains("relay fact 4"), "{own}");

    // Three open threads, then two opened from the relay's activities.
    let mut open = Vec::new();
    for n in 0..3 {
        open.push(open_thread(&app, &session, &format!("Open {n}")).await);
        sleep(StdDuration::from_millis(5)).await;
    }
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(rider.user_id, &rider.tenant, RELAY, &sessions())
        .await
        .unwrap();
    for activity in ["g1", "g2"] {
        let thread = open_thread(&app, &session, activity).await;
        let (status, linked) = send_json(
            &app,
            "PUT",
            &format!("/api/me/activities/{RELAY}/{activity}/conversation"),
            &session,
            &serde_json::json!({ "conversation_id": thread }),
        )
        .await;
        assert!(status.is_success(), "{linked}");
        sleep(StdDuration::from_millis(5)).await;
    }
    let (status, page) = get(&app, "/api/chat/conversations?limit=3", &key).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let mut ids: Vec<String> = page["conversations"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|c| c["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    open.sort();
    assert_eq!(
        ids, open,
        "the key's page holds the three open threads: {page}"
    );
    assert_eq!(page["total"], 3, "{page}");
    let (_, own_page) = get(&app, "/api/chat/conversations?limit=3", &session).await;
    assert_eq!(own_page["total"], 5, "{own_page}");
}

#[tokio::test]
async fn a_room_entry_copied_from_a_stamped_reply_keeps_its_place_withheld_over_an_api_key() {
    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;
    let repos = &resources.common.repos;
    let agent = repos
        .agents
        .create(
            rider.user_id,
            rider.tenant,
            &CreateAgentRequest {
                title: "Room Coach".to_owned(),
                description: None,
                system_prompt: "You coach the room.".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();
    let now = Utc::now();
    let group_id = Uuid::new_v4();
    repos
        .groups
        .create_group(
            rider.tenant,
            &CoachingGroup {
                id: group_id,
                tenant_id: rider.tenant.to_string(),
                name: "Relay Room".to_owned(),
                description: None,
                agent_id: agent.id.to_string(),
                owner_id: rider.user_id,
                coach_user_id: None,
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::Mentions,
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id: rider.user_id,
            tenant_id: rider.tenant.to_string(),
            role: GroupRole::Owner,
            peer_sharing_consent: true,
            coach_sharing_consent: true,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
    for (content, transport_policy) in [
        (
            "Open room note: hydrate well.",
            TransportPolicy::AnyTransport,
        ),
        (
            "Relay-derived room reply: Garmin Hills splits.",
            TransportPolicy::FirstPartyOnly,
        ),
    ] {
        repos
            .groups
            .append_transcript_entry(&NewGroupTranscriptEntry {
                group_id: &group_id.to_string(),
                tenant_id: &rider.tenant.to_string(),
                author_user_id: rider.user_id,
                speaker: TranscriptSpeaker::Coach,
                content,
                source_conversation_id: None,
                source_message_id: None,
                transport_policy,
            })
            .await
            .unwrap();
    }
    let room = format!("/api/chat/groups/{group_id}/transcript");

    let (status, external) = get(&app, &room, &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let text = external.to_string();
    assert!(text.contains("Open room note"), "{text}");
    assert!(!text.contains("Garmin Hills"), "{text}");
    assert_eq!(
        external["entries"].as_array().map_or(0, Vec::len),
        2,
        "the stamped entry keeps its place in the room: {external}"
    );

    let (status, own) = get(&app, &room, &session).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert!(own.to_string().contains("Garmin Hills"), "{own}");
}

#[cfg(feature = "client-notifications")]
#[tokio::test]
async fn the_feed_withholds_a_stamped_notification_and_devices_register_from_the_app_only() {
    use pierre_notifications::events::stamp_data;
    use pierre_notifications::models::NotificationCategory;
    use pierre_notifications::{DispatchRequest, PushTier, TenantId as CommTenantId};

    let resources = server_with_relay().await;
    let rider = athlete_with_relay(&resources).await;
    let app = ProviderToolRouter::build_http_app(&resources);
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &rider.user).await
    );
    let key = api_key(&resources, rider.user_id).await;
    let service = resources
        .common
        .notification_service
        .clone()
        .expect("the test server wires notifications");

    for (title, policy) in [
        ("Open reminder", TransportPolicy::AnyTransport),
        (
            "Relay record on Garmin Hills",
            TransportPolicy::FirstPartyOnly,
        ),
    ] {
        service
            .dispatch_with_tier(
                &DispatchRequest {
                    user_id: rider.user_id,
                    tenant_id: CommTenantId(rider.tenant.as_uuid()),
                    category: NotificationCategory::System,
                    notification_type: "system_notice".to_owned(),
                    title: title.to_owned(),
                    body: title.to_owned(),
                    data: stamp_data(None, policy),
                    image_url: None,
                    actions: None,
                    bypass_frequency_cap: true,
                },
                PushTier::P0,
            )
            .await
            .unwrap();
    }

    let (status, external) = get(&app, "/api/notifications", &key).await;
    assert_eq!(status, StatusCode::OK, "{external}");
    let text = external.to_string();
    assert!(text.contains("Open reminder"), "{text}");
    assert!(!text.contains("Garmin Hills"), "{text}");
    assert_eq!(external["total"], 1, "{external}");
    let (_, own) = get(&app, "/api/notifications", &session).await;
    assert!(own.to_string().contains("Garmin Hills"), "{own}");

    let device = serde_json::json!({
        "expo_push_token": "ExponentPushToken[transport-gate]",
        "platform": "ios",
    });
    let (status, refused) =
        send_json(&app, "POST", "/api/notifications/device", &key, &device).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(refused["code"], "UnavailableOverTransport", "{refused}");
    let (status, registered) =
        send_json(&app, "POST", "/api/notifications/device", &session, &device).await;
    assert!(status.is_success(), "{registered}");
}

#[tokio::test]
async fn an_athlete_without_a_first_party_only_connection_keeps_api_key_access() {
    let resources = server_with_relay().await;
    let email = format!("open-{}@example.com", Uuid::new_v4());
    let (user_id, _, _) = create_test_user_with_plan(&resources.agent.database, &email, "starter")
        .await
        .unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let key = api_key(&resources, user_id).await;

    for uri in ["/api/chat/conversations", "/api/memory/facts"] {
        let (status, body) = get(&app, uri, &key).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    }
}
