// ABOUTME: carnet#1262 — the coach's roster keeps a provider out while the athlete withholds its AI consent
// ABOUTME: GET /api/coach/roster and get_roster_overview: shared + WHOOP → withdrawn, no WHOOP figure → given back

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sharing with a coach and consenting to AI use of a provider's data are two
//! decisions. JF decided on 2026-10-30 (carnet#1262) that the roster read
//! keeps the AI-consent gate for the human coach too, so the REST read and the
//! agent's tool stay one projection: an athlete who shares with their coach
//! (`coach_sharing_consent`) but withdrew their WHOOP AI consent is still a
//! `shared` row, and that row carries no figure WHOOP produced. Giving the
//! consent back brings the figure back.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate, and the surviving crate doc keeps `missing_docs` quiet.
#![cfg(all(feature = "provider-whoop", feature = "tools-groups"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use chrono::{Duration, Utc};
use pierre_core::ai_policy::SourcePolicy;
use pierre_core::constants::oauth::providers::WHOOP;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    Activity, ActivityBuilder, AgentCategory, Athlete, ConnectionType, CreateAgentRequest,
    SportType, Stats, TenantId, UserOAuthToken,
};
use pierre_core::pagination::{CursorPage, PaginationParams};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_roster::CoachRosterRoutes;
use pierre_providers::core::{
    ActivityQueryParams, FitnessProvider, OAuth2Credentials, ProviderConfig,
};
use pierre_providers::provider_terms::WHOOP as WHOOP_POLICY;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::spi::{
    OAuthEndpoints, OAuthParams, OAuthRefresh, ProviderBundle, ProviderCapabilities,
    ProviderDescriptor,
};
use pierre_routes_auth::AuthRoutes;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use uuid::Uuid;

use crate::common::{
    create_test_server_resources, create_test_user_with_plan, generate_test_token,
};
use crate::helpers::axum_test::AxumTestRequest;

/// The one WHOOP workout the fake account holds.
const WORKOUT: &str = "WHOOP Morning Ride";

/// The athlete's display name, as the tool's roster names them.
const ATHLETE: &str = "Alex Athlete";

/// WHOOP as the registry declares it, served by a fake account so the test
/// reaches no network.
struct WhoopDescriptor;

impl ProviderDescriptor for WhoopDescriptor {
    fn name(&self) -> &'static str {
        WHOOP
    }

    fn display_name(&self) -> &'static str {
        "WHOOP"
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
        "http://localhost/whoop"
    }

    fn default_scopes(&self) -> &'static [&'static str] {
        &[]
    }

    fn ai_policy(&self) -> &'static SourcePolicy {
        &WHOOP_POLICY
    }
}

struct FakeWhoop {
    config: ProviderConfig,
}

fn workouts() -> Vec<Activity> {
    vec![ActivityBuilder::new(
        "w1",
        WORKOUT,
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        WHOOP,
    )
    .average_heart_rate(141)
    .build()]
}

#[async_trait]
impl FitnessProvider for FakeWhoop {
    fn name(&self) -> &'static str {
        WHOOP
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
        Err(AppError::internal("not read by this test"))
    }

    async fn get_activities_with_params(
        &self,
        _params: &ActivityQueryParams,
    ) -> AppResult<Vec<Activity>> {
        Ok(workouts())
    }

    async fn get_activities_cursor(
        &self,
        _params: &PaginationParams,
    ) -> AppResult<CursorPage<Activity>> {
        Ok(CursorPage::new(workouts(), None, None, false))
    }

    async fn get_activity(&self, id: &str) -> AppResult<Activity> {
        workouts()
            .into_iter()
            .find(|a| a.id() == id)
            .ok_or_else(|| AppError::not_found(id.to_owned()))
    }

    async fn get_stats(&self) -> AppResult<Stats> {
        Err(AppError::internal("not read by this test"))
    }
}

fn whoop_factory(config: ProviderConfig) -> Box<dyn FitnessProvider> {
    Box::new(FakeWhoop { config })
}

async fn server_with_fake_whoop() -> Arc<ServerContext> {
    let base = create_test_server_resources().await.unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register_provider_bundle(ProviderBundle::new(
        Box::new(WhoopDescriptor),
        whoop_factory,
    ));
    let mut context = (*base).clone();
    context.fitness.provider_registry = Arc::new(registry);
    Arc::new(context)
}

struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    res.common
        .repos
        .users
        .update_display_name(id, name)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    Person { id, tenant, auth }
}

/// A coach holds `manages_roster`, as attaching them to a group requires.
async fn coach(res: &Arc<ServerContext>, email: &str) -> Person {
    let coach = person(res, email, "Casey Coach").await;
    res.common
        .repos
        .users
        .set_manages_roster(coach.id, true)
        .await
        .unwrap();
    coach
}

/// A group in the coach's tenant whose human coach is `coach`.
async fn coached_group(res: &Arc<ServerContext>, coach: &Person) -> Uuid {
    let agent_id = res
        .common
        .repos
        .agents
        .create(
            coach.id,
            coach.tenant,
            &CreateAgentRequest {
                title: "Group Agent".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
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
        .unwrap()
        .id
        .to_string();
    let now = Utc::now();
    res.common
        .repos
        .groups
        .create_group(
            coach.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: coach.tenant.to_string(),
                name: "Squad".to_owned(),
                description: None,
                agent_id,
                owner_id: coach.id,
                coach_user_id: Some(coach.id),
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 100,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id
}

/// `athlete` joins `group` sharing their training with its coach.
async fn join_sharing(res: &Arc<ServerContext>, group: Uuid, athlete: &Person) {
    let now = Utc::now();
    res.common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group,
            user_id: athlete.id,
            tenant_id: athlete.tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// WHOOP connected in the athlete's tenant, its workout already cached.
async fn connect_whoop(res: &Arc<ServerContext>, athlete: &Person) {
    let repos = &res.common.repos;
    let token = UserOAuthToken::new(
        athlete.id,
        athlete.tenant.to_string(),
        WHOOP.to_owned(),
        "whoop-access".to_owned(),
        Some("whoop-refresh".to_owned()),
        Some(Utc::now() + Duration::hours(6)),
        None,
    );
    repos.oauth_tokens.upsert_token(&token).await.unwrap();
    repos
        .provider_connections
        .register_connection(
            athlete.id,
            athlete.tenant,
            WHOOP,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(athlete.id, &athlete.tenant, WHOOP, &workouts())
        .await
        .unwrap();
}

/// `PUT` (give) or `DELETE` (withdraw) the athlete's WHOOP AI consent.
async fn set_whoop_consent(res: &Arc<ServerContext>, athlete: &Person, give: bool) {
    let path = format!("/api/providers/{WHOOP}/ai-consent");
    let request = if give {
        AxumTestRequest::put(&path)
    } else {
        AxumTestRequest::delete(&path)
    };
    let resp = request
        .header("authorization", &athlete.auth)
        .send(AuthRoutes::routes(res.auth_routes_context()))
        .await;
    let status = resp.status();
    let body = resp.text();
    assert_eq!(status, 200, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["ai_consent"], give, "{body}");
}

fn router(res: &Arc<ServerContext>) -> Router {
    CoachRosterRoutes::routes(Arc::clone(res))
}

/// The athlete's row on the coach's `GET /api/coach/roster`.
async fn rest_row(res: &Arc<ServerContext>, coach: &Person, athlete: &Person) -> Value {
    let response = AxumTestRequest::get("/api/coach/roster")
        .header("authorization", &coach.auth)
        .send(router(res))
        .await;
    let body = response.body_text();
    assert_eq!(response.status(), 200, "{body}");
    let page: Value = serde_json::from_str(&body).unwrap();
    page["athletes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["user_id"] == athlete.id.to_string())
        .unwrap_or_else(|| panic!("the athlete is on the roster: {page:#}"))
        .clone()
}

/// The athlete's row in `get_roster_overview`, run by the real executor as
/// the coach's agent runs it.
async fn tool_row(res: &Arc<ServerContext>, coach: &Person) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(res);
    let response = UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .execute_tool(UniversalRequest {
            tool_name: "get_roster_overview".to_owned(),
            parameters: serde_json::json!({}),
            user_id: coach.id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(coach.tenant.to_string()),
        })
        .await
        .expect("the call completes");
    assert!(response.success, "{:?}", response.error);
    let result = response.result.expect("a payload");
    result["athletes"]
        .as_array()
        .unwrap_or_else(|| panic!("a roster: {result:#}"))
        .iter()
        .find(|a| a["name"] == ATHLETE)
        .unwrap_or_else(|| panic!("the athlete is on the tool's roster: {result:#}"))
        .clone()
}

/// The REST row carries the WHOOP ride: its source, its day, its session.
fn assert_rest_reads_whoop(row: &Value) {
    assert_eq!(row["status"], "shared", "{row:#}");
    let training = &row["training"];
    assert!(
        training["last_activity_per_source"].get(WHOOP).is_some(),
        "WHOOP delivered the last session: {row:#}"
    );
    assert_eq!(training["last_session"]["sport"], "Ride", "{row:#}");
    assert_eq!(training["days_since_last_activity"], 1, "{row:#}");
}

/// The REST row is still `shared`, but carries nothing WHOOP produced.
fn assert_rest_withholds_whoop(row: &Value) {
    assert_eq!(
        row["status"], "shared",
        "withdrawing AI consent never touches coach sharing: {row:#}"
    );
    let training = &row["training"];
    assert!(
        training["last_activity_per_source"].get(WHOOP).is_none(),
        "no WHOOP source on the row: {row:#}"
    );
    assert!(
        training["last_session"].is_null(),
        "no WHOOP session on the row: {row:#}"
    );
    assert!(
        training["days_since_last_activity"].is_null(),
        "no day counted from a WHOOP session: {row:#}"
    );
}

#[tokio::test]
async fn a_withdrawn_ai_consent_keeps_whoop_off_the_coachs_rest_roster() {
    let res = server_with_fake_whoop().await;
    let casey = coach(&res, "casey@roster-ai-consent.test").await;
    let alex = person(&res, "alex@roster-ai-consent.test", ATHLETE).await;
    let group = coached_group(&res, &casey).await;
    join_sharing(&res, group, &alex).await;
    connect_whoop(&res, &alex).await;

    set_whoop_consent(&res, &alex, true).await;
    assert_rest_reads_whoop(&rest_row(&res, &casey, &alex).await);

    set_whoop_consent(&res, &alex, false).await;
    assert_rest_withholds_whoop(&rest_row(&res, &casey, &alex).await);

    set_whoop_consent(&res, &alex, true).await;
    assert_rest_reads_whoop(&rest_row(&res, &casey, &alex).await);
}

#[tokio::test]
async fn a_withdrawn_ai_consent_keeps_whoop_out_of_get_roster_overview() {
    let res = server_with_fake_whoop().await;
    let casey = coach(&res, "casey@tool-ai-consent.test").await;
    let alex = person(&res, "alex@tool-ai-consent.test", ATHLETE).await;
    let group = coached_group(&res, &casey).await;
    join_sharing(&res, group, &alex).await;
    connect_whoop(&res, &alex).await;

    set_whoop_consent(&res, &alex, true).await;
    let given = tool_row(&res, &casey).await;
    assert_eq!(given["status"], "shared", "{given:#}");
    assert!(given.to_string().contains(WORKOUT), "{given:#}");
    assert!(
        given["training"]["last_activity_per_source"]
            .get(WHOOP)
            .is_some(),
        "{given:#}"
    );

    set_whoop_consent(&res, &alex, false).await;
    let withdrawn = tool_row(&res, &casey).await;
    assert_eq!(
        withdrawn["status"], "shared",
        "withdrawing AI consent never touches coach sharing: {withdrawn:#}"
    );
    assert!(
        !withdrawn.to_string().contains(WORKOUT),
        "no WHOOP session reaches the coach's agent: {withdrawn:#}"
    );
    assert!(
        withdrawn["training"]["last_activity_per_source"]
            .get(WHOOP)
            .is_none(),
        "no WHOOP source on the row: {withdrawn:#}"
    );

    set_whoop_consent(&res, &alex, true).await;
    let restored = tool_row(&res, &casey).await;
    assert!(restored.to_string().contains(WORKOUT), "{restored:#}");
}

/// The athletes' names in the coach's roster order.
async fn rest_order(res: &Arc<ServerContext>, coach: &Person) -> Vec<String> {
    let response = AxumTestRequest::get("/api/coach/roster")
        .header("authorization", &coach.auth)
        .send(router(res))
        .await;
    let page: Value = serde_json::from_str(&response.body_text()).unwrap();
    page["athletes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_owned())
        .collect()
}

/// The order puts the longest without a session first, so a withheld WHOOP
/// session must not place the athlete either: its day would show through the
/// order while the row carries none.
#[tokio::test]
async fn a_withdrawn_ai_consent_no_longer_places_the_athlete_in_the_roster_order() {
    let res = server_with_fake_whoop().await;
    let casey = coach(&res, "casey@order-ai-consent.test").await;
    let alex = person(&res, "alex@order-ai-consent.test", ATHLETE).await;
    let ben = person(&res, "ben@order-ai-consent.test", "Ben Athlete").await;
    let group = coached_group(&res, &casey).await;
    join_sharing(&res, group, &alex).await;
    join_sharing(&res, group, &ben).await;
    connect_whoop(&res, &alex).await;
    // Ben rode three days ago, on a source with no AI consent to withhold.
    let repos = &res.common.repos;
    repos
        .provider_connections
        .register_connection(ben.id, ben.tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let ride = ActivityBuilder::new(
        "b1",
        "Ben's ride",
        SportType::Ride,
        Utc::now() - Duration::days(3),
        3_600,
        "strava",
    )
    .build();
    repos
        .activity_cache
        .upsert_activities(ben.id, &ben.tenant, "strava", &[ride])
        .await
        .unwrap();

    set_whoop_consent(&res, &alex, true).await;
    assert_eq!(
        rest_order(&res, &casey).await,
        ["Ben Athlete", ATHLETE],
        "Alex rode yesterday, Ben three days ago"
    );

    set_whoop_consent(&res, &alex, false).await;
    assert_eq!(
        rest_order(&res, &casey).await,
        [ATHLETE, "Ben Athlete"],
        "with WHOOP withheld, Alex has no session the coach may see"
    );
}

/// A relay caches a WHOOP session under its own key, with WHOOP as its
/// `source`. Once WHOOP is withheld, that copy must not place the athlete
/// either: only a session neither from nor relayed from WHOOP does.
#[tokio::test]
async fn a_withheld_session_a_relay_cached_does_not_place_the_athlete() {
    let res = server_with_fake_whoop().await;
    let casey = coach(&res, "casey@relay-ai-consent.test").await;
    let alex = person(&res, "alex@relay-ai-consent.test", ATHLETE).await;
    let ben = person(&res, "ben@relay-ai-consent.test", "Ben Athlete").await;
    let group = coached_group(&res, &casey).await;
    join_sharing(&res, group, &alex).await;
    join_sharing(&res, group, &ben).await;
    connect_whoop(&res, &alex).await;
    let repos = &res.common.repos;
    let cache = |user: &Person, provider: &'static str, activity: Activity| {
        let repos = Arc::clone(repos);
        let (id, tenant) = (user.id, user.tenant);
        async move {
            repos
                .provider_connections
                .register_connection(id, tenant, provider, &ConnectionType::OAuth, None)
                .await
                .unwrap();
            repos
                .activity_cache
                .upsert_activities(id, &tenant, provider, &[activity])
                .await
                .unwrap();
        }
    };
    // Alex: the relay's copy of yesterday's WHOOP session, and a Strava run
    // five days ago. Ben: a Strava ride three days ago.
    let relayed = ActivityBuilder::new(
        "r1",
        "Relayed WHOOP session",
        SportType::Ride,
        Utc::now() - Duration::days(1),
        3_600,
        "intervals_icu",
    )
    .source("WHOOP")
    .build();
    cache(&alex, "intervals_icu", relayed).await;
    let run = ActivityBuilder::new(
        "a5",
        "Alex's run",
        SportType::Run,
        Utc::now() - Duration::days(5),
        1_800,
        "strava",
    )
    .build();
    cache(&alex, "strava", run).await;
    let ride = ActivityBuilder::new(
        "b3",
        "Ben's ride",
        SportType::Ride,
        Utc::now() - Duration::days(3),
        3_600,
        "strava",
    )
    .build();
    cache(&ben, "strava", ride).await;

    set_whoop_consent(&res, &alex, true).await;
    assert_eq!(
        rest_order(&res, &casey).await,
        ["Ben Athlete", ATHLETE],
        "with consent, Alex last trained yesterday"
    );

    set_whoop_consent(&res, &alex, false).await;
    assert_eq!(
        rest_order(&res, &casey).await,
        [ATHLETE, "Ben Athlete"],
        "with WHOOP withheld, Alex last trained five days ago, relay or not"
    );
}

/// Past a full page of withheld sessions, the order reads on — and a session
/// sharing the instant the page ended on is read, not stepped over: an
/// athlete's WHOOP and Strava copies of one ride start together.
#[tokio::test]
async fn the_order_reads_past_a_page_of_withheld_sessions_and_its_last_instant() {
    let res = server_with_fake_whoop().await;
    let casey = coach(&res, "casey@paged-ai-consent.test").await;
    let alex = person(&res, "alex@paged-ai-consent.test", ATHLETE).await;
    let ben = person(&res, "ben@paged-ai-consent.test", "Ben Athlete").await;
    let group = coached_group(&res, &casey).await;
    join_sharing(&res, group, &alex).await;
    join_sharing(&res, group, &ben).await;
    connect_whoop(&res, &alex).await;
    let repos = &res.common.repos;
    let ride = |id: &str, start, provider: &str| {
        ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, provider).build()
    };
    // Alex's ride two days ago, cached by WHOOP with two more sessions at the
    // same instant and 98 since — with `connect_whoop`'s ride, 99 newer than
    // it, so the first page of 100 ends on that instant with its other
    // sessions still unread — and then by Strava, at that instant too.
    let shared_start = Utc::now() - Duration::days(2);
    for user in [&alex, &ben] {
        repos
            .provider_connections
            .register_connection(user.id, user.tenant, "strava", &ConnectionType::OAuth, None)
            .await
            .unwrap();
    }
    let whoop: Vec<Activity> = (0..3)
        .map(|i| ride(&format!("t{i}"), shared_start, WHOOP))
        .chain((1..=98).map(|i| ride(&format!("n{i}"), shared_start + Duration::minutes(i), WHOOP)))
        .collect();
    repos
        .activity_cache
        .upsert_activities(alex.id, &alex.tenant, WHOOP, &whoop)
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(
            alex.id,
            &alex.tenant,
            "strava",
            &[ride("s1", shared_start, "strava")],
        )
        .await
        .unwrap();
    // Ben rode six days ago.
    repos
        .activity_cache
        .upsert_activities(
            ben.id,
            &ben.tenant,
            "strava",
            &[ride("b6", Utc::now() - Duration::days(6), "strava")],
        )
        .await
        .unwrap();

    set_whoop_consent(&res, &alex, false).await;
    assert_eq!(
        rest_order(&res, &casey).await,
        ["Ben Athlete", ATHLETE],
        "with WHOOP withheld, Alex last trained two days ago on Strava, Ben six"
    );
}
