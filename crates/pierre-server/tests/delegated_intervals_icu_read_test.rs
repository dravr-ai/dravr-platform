// ABOUTME: An Intervals.icu coach links roster athletes to group members, who then read through the coach's API key
// ABOUTME: Pins the roster read, the athlete path and key on the wire, the provider card, and how each refusal ends or flags the link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! An Intervals.icu coach reads every athlete who shares with them through
//! their own API key: `GET /api/v1/athletes` lists them, and each athlete is
//! read at their own path. A group's coach lists that roster and proposes
//! which athlete is which member, over REST; the member confirms. From then
//! on the member's reads go through the coach's key with the member's
//! athlete id on the wire.
//!
//! The refusals are the other half of the contract:
//!
//! - an Intervals.icu OAuth grant lists no athletes, so the roster asks the
//!   coach for their API key;
//! - an activity of another athlete is not found through the link;
//! - a dead coach key flags the coach, never the member;
//! - an athlete who stops sharing with the coach ends the link.
//!
//! One test, because the Intervals.icu base URL is read from the environment
//! when the server's provider registry is built. The API is a loopback
//! stand-in (a test double, per the repo's mock rule) that answers by the
//! key and the path each request names and records every request.

mod common;
mod helpers;

use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionStatus, ConnectionType, CreateAgentRequest, DelegatedConnection,
    DelegationEndReason, DelegationStatus, TenantId, UserOAuthToken, API_KEY_TOKEN_TYPE,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::delegation::{is_athlete_off_roster, is_coach_credential_expired};
use pierre_providers::errors::ErrorCode;
use pierre_providers::CoreFitnessProvider;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::{DelegatedConnectionRoutes, NotificationRoutes};
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// The coach's live API key.
const COACH_KEY: &str = "coach-key";
/// The coach's key once Intervals.icu stops honouring it.
const DEAD_KEY: &str = "dead-key";
const COACH_NAME: &str = "Casey Coach";

/// The member the reads are pinned on, and their athlete id.
const M1_ATHLETE: &str = "i201";
/// The member who stops sharing with the coach.
const M2_ATHLETE: &str = "i202";

/// The one activity on the linked member's calendar.
const M1_ACTIVITY: &str = "a1";
/// An activity of the second athlete, which the first member's link must
/// never reach.
const M2_ACTIVITY: &str = "a2";

/// How long a spawned step (the warm-up after a confirm) gets to land.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Hit {
    path: String,
    key: String,
}

/// The stand-in's record and its one switch.
#[derive(Clone, Default)]
struct Api {
    hits: Arc<Mutex<Vec<Hit>>>,
    /// The second athlete no longer shares with the coach.
    drop_second: Arc<AtomicBool>,
}

impl Api {
    /// Record a request to `path`, returning whether its key is the coach's
    /// live one.
    fn record(&self, path: &str, headers: &HeaderMap) -> bool {
        let key = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Basic "))
            .and_then(|encoded| STANDARD.decode(encoded).ok())
            .and_then(|pair| String::from_utf8(pair).ok())
            .and_then(|pair| pair.strip_prefix("API_KEY:").map(str::to_owned))
            .unwrap_or_default();
        let live = key == COACH_KEY;
        self.hits.lock().unwrap().push(Hit {
            path: path.to_owned(),
            key,
        });
        live
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.hits.lock().unwrap().clear();
    }
}

fn unauthorized() -> (StatusCode, Json<Value>) {
    (StatusCode::UNAUTHORIZED, Json(json!({})))
}

fn activity(id: &str, athlete: &str) -> Value {
    json!({
        "id": id,
        "name": "Easy run",
        "type": "Run",
        "start_date_local": (Utc::now() - ChronoDuration::days(2))
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string(),
        "elapsed_time": 1800,
        "distance": 5000.0,
        "icu_athlete_id": athlete,
    })
}

async fn athletes_route(State(api): State<Api>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !api.record("/api/v1/athletes", &headers) {
        return unauthorized();
    }
    // Each athlete carries the email of the member they are, which is what a
    // link binds by; the coach's own entry binds the key to the coach.
    let mut roster = vec![
        json!({ "id": COACH_ATHLETE, "name": COACH_NAME, "email": "coach@intervals-read.test" }),
        json!({ "id": M1_ATHLETE, "name": "Alex Athlete", "email": "m1@intervals-read.test" }),
    ];
    if !api.drop_second.load(Ordering::SeqCst) {
        roster.push(
            json!({ "id": M2_ATHLETE, "name": "Sam Swimmer", "email": "m2@intervals-read.test" }),
        );
    }
    (StatusCode::OK, Json(Value::Array(roster)))
}

async fn athlete_activities_route(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> (StatusCode, Json<Value>) {
    if !api.record(&format!("/api/v1/athlete/{athlete}/activities"), &headers) {
        return unauthorized();
    }
    match athlete.as_str() {
        M1_ATHLETE => (
            StatusCode::OK,
            Json(json!([activity(M1_ACTIVITY, M1_ATHLETE)])),
        ),
        M2_ATHLETE if !api.drop_second.load(Ordering::SeqCst) => (StatusCode::OK, Json(json!([]))),
        _ => (StatusCode::FORBIDDEN, Json(json!({}))),
    }
}

async fn activity_route(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    if !api.record(&format!("/api/v1/activity/{id}"), &headers) {
        return unauthorized();
    }
    match id.as_str() {
        M1_ACTIVITY => (StatusCode::OK, Json(activity(M1_ACTIVITY, M1_ATHLETE))),
        M2_ACTIVITY => (StatusCode::OK, Json(activity(M2_ACTIVITY, M2_ATHLETE))),
        _ => (StatusCode::NOT_FOUND, Json(json!({}))),
    }
}

async fn spawn_api(api: Api) -> String {
    let app = Router::new()
        .route("/api/v1/athletes", get(athletes_route))
        .route(
            "/api/v1/athlete/{athlete}/activities",
            get(athlete_activities_route),
        )
        .route("/api/v1/activity/{id}", get(activity_route))
        .with_state(api);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

/// One signed-in user.
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

struct World {
    res: Arc<ServerContext>,
    runtime: Arc<dyn ToolRuntime>,
    router: Router,
    api: Api,
    group: Uuid,
    coach: Person,
    m1: Person,
    m2: Person,
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
    // The notices are read back in the language they are asserted in.
    res.common
        .repos
        .users
        .update_locale(id, "en")
        .await
        .unwrap();
    // A verified email: what binds the coach's key, and a roster athlete, to
    // this person.
    res.common
        .repos
        .email_verification
        .mark_verified(id)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    Person { id, tenant, auth }
}

/// The coach's stored Intervals.icu credential: their API key, or with
/// `token_type` another kind of grant.
fn coach_token(coach: &Person, key: &str, token_type: &str) -> UserOAuthToken {
    let now = Utc::now();
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id: coach.id,
        tenant_id: coach.tenant.to_string(),
        provider: INTERVALS_ICU.to_owned(),
        access_token: key.to_owned(),
        refresh_token: None,
        token_type: token_type.to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: Some(COACH_ATHLETE.to_owned()),
        oauth_app_client_id: None,
        created_at: now,
        updated_at: now,
    }
}

fn agent_request() -> CreateAgentRequest {
    CreateAgentRequest {
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
    }
}

async fn world(api: Api) -> World {
    let res = create_test_server_resources().await.unwrap();
    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@intervals-read.test", "Olive Owner").await;
    let coach = person(&res, "coach@intervals-read.test", COACH_NAME).await;
    let m1 = person(&res, "m1@intervals-read.test", "Alex Athlete").await;
    let m2 = person(&res, "m2@intervals-read.test", "Sam Swimmer").await;

    let agent_id = repos
        .agents
        .create(owner.id, owner.tenant, &agent_request())
        .await
        .unwrap()
        .id
        .to_string();
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: "Squad".to_owned(),
                description: None,
                agent_id,
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
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
        .unwrap()
        .id;
    assert!(repos
        .groups
        .set_group_coach_user(&group.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    for member in [&m1, &m2] {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group,
                user_id: member.id,
                tenant_id: member.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    // The coach's own Intervals.icu API key.
    repos
        .oauth_tokens
        .upsert_token(&coach_token(&coach, COACH_KEY, API_KEY_TOKEN_TYPE))
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            coach.id,
            coach.tenant,
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = res.clone();
    let router = DelegatedConnectionRoutes::routes(Arc::clone(&res))
        .merge(NotificationRoutes::routes(Arc::clone(&res)))
        .merge(AuthRoutes::routes(res.auth_routes_context()));
    World {
        res,
        runtime,
        router,
        api,
        group,
        coach,
        m1,
        m2,
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

impl World {
    fn links_path(&self) -> String {
        format!("/api/groups/{}/delegated-connections", self.group)
    }

    async fn get(&self, path: &str, who: &Person) -> (StatusCode, Value) {
        let resp = AxumTestRequest::get(path)
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    async fn post(&self, path: &str, who: &Person, body: &Value) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post(path)
            .header("authorization", &who.auth)
            .json(body)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    /// The coach proposes `athlete` as `member`, the member confirms, and
    /// the warm-up read the confirm spawns lands. Returns the link id.
    async fn link(&self, athlete: &str, member: &Person) -> Uuid {
        let (status, proposed) = self
            .post(
                &self.links_path(),
                &self.coach,
                &json!({
                    "provider": "intervals_icu",
                    "provider_athlete_id": athlete,
                    "member_user_id": member.id,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{proposed}");
        assert_eq!(proposed["provider"], "intervals_icu");
        let id = proposed["id"].as_str().unwrap().to_owned();

        let (status, confirmed) = self
            .post(
                &format!("{}/{id}/confirm", self.links_path()),
                member,
                &json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        assert_eq!(confirmed["status"], "confirmed");

        // The confirm reads the member's recent activities off the request,
        // at the member's path, with the coach's key.
        let path = format!("/api/v1/athlete/{athlete}/activities");
        let deadline = Instant::now() + SPAWN_DEADLINE;
        while !self
            .api
            .hits()
            .iter()
            .any(|h| h.path == path && h.key == COACH_KEY)
        {
            assert!(Instant::now() < deadline, "the warm-up read never happened");
            sleep(Duration::from_millis(50)).await;
        }
        Uuid::parse_str(&id).unwrap()
    }

    /// The feed row of `wire` type `who` reads, waiting for the
    /// fire-and-forget dispatch to land.
    async fn notice(&self, who: &Person, wire: &str) -> Value {
        let deadline = Instant::now() + SPAWN_DEADLINE;
        loop {
            let (status, body) = self.get("/api/notifications", who).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            if let Some(row) = body["data"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["notification_type"] == wire)
            {
                return row.clone();
            }
            assert!(Instant::now() < deadline, "no {wire} notice landed");
            sleep(Duration::from_millis(100)).await;
        }
    }

    /// The link as stored, read as its coach.
    async fn stored(&self, link: Uuid) -> DelegatedConnection {
        self.res
            .common
            .repos
            .delegated_connections
            .get_for_participant(link, self.group, self.coach.id)
            .await
            .unwrap()
            .expect("an ended link stays for audit")
    }

    async fn connection(&self, who: &Person) -> Option<(ConnectionType, ConnectionStatus)> {
        self.res
            .common
            .repos
            .provider_connections
            .get_for_user(who.id, Some(who.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == INTERVALS_ICU)
            .map(|c| (c.connection_type, c.status))
    }

    /// The Intervals.icu card `who` reads on `/api/providers`.
    async fn intervals_card(&self, who: &Person) -> Value {
        let (_, body) = self.get("/api/providers", who).await;
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["provider"] == INTERVALS_ICU)
            .cloned()
            .expect("the Intervals.icu card is listed")
    }

    /// The provider `who`'s Intervals.icu reads are served by, or the
    /// refusal's words.
    async fn authenticate(&self, who: &Person) -> Result<Box<dyn CoreFitnessProvider>, String> {
        AuthService::new(Arc::clone(&self.runtime))
            .create_authenticated_provider(INTERVALS_ICU, who.id, Some(&who.tenant.to_string()))
            .await
            .map_err(|response| response.error.unwrap_or_default())
    }

    async fn head(&self, who: &Person) -> AppResult<usize> {
        let params = ActivityQueryParams {
            limit: Some(10),
            offset: None,
            before: None,
            after: None,
        };
        fetch_provider_head(
            &self.runtime,
            INTERVALS_ICU,
            who.id,
            &who.tenant.to_string(),
            &params,
        )
        .await
        .map(|activities| activities.len())
    }
}

/// The coach reads the roster their key lists, the coach's own entry left
/// out, with or without naming the platform: it is the one they connected.
async fn the_coach_reads_their_intervals_roster(w: &World) {
    for path in [
        format!("{}/roster?provider=intervals_icu", w.links_path()),
        format!("{}/roster", w.links_path()),
    ] {
        let (status, body) = w.get(&path, &w.coach).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["provider"], "intervals_icu", "{body}");
        let ids: Vec<&str> = body["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["provider_athlete_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [M1_ATHLETE, M2_ATHLETE], "{body}");
    }
    let roster_reads: Vec<Hit> = w
        .api
        .hits()
        .into_iter()
        .filter(|h| h.path == "/api/v1/athletes")
        .collect();
    assert!(!roster_reads.is_empty());
    assert!(roster_reads.iter().all(|h| h.key == COACH_KEY));
}

/// The member's reads go through the coach's key at the member's path, and
/// the card names the link.
async fn the_member_reads_through_the_coachs_key(w: &World) {
    w.api.clear();
    assert_eq!(w.head(&w.m1).await.unwrap(), 1);
    let hits = w.api.hits();
    assert!(
        hits.iter()
            .all(|h| h.key == COACH_KEY
                && h.path == format!("/api/v1/athlete/{M1_ATHLETE}/activities")),
        "{hits:?}"
    );

    let provider = w
        .authenticate(&w.m1)
        .await
        .expect("served through the coach");
    let detail = provider
        .get_activity(M1_ACTIVITY)
        .await
        .expect("the member's own activity reads");
    assert_eq!(detail.id(), M1_ACTIVITY);
    let refused = provider
        .get_activity(M2_ACTIVITY)
        .await
        .expect_err("another athlete's activity is outside the link");
    assert_eq!(refused.code, ErrorCode::ResourceNotFound, "{refused}");

    let card = w.intervals_card(&w.m1).await;
    assert_eq!(card["connected"], true, "{card}");
    assert_eq!(card["delegation"]["status"], "confirmed", "{card}");
    assert_eq!(card["delegation"]["coach_display_name"], COACH_NAME);
    assert_eq!(card["delegation"]["coach_needs_reauth"], false);
    assert_eq!(
        w.connection(&w.m1).await,
        Some((ConnectionType::Delegated, ConnectionStatus::Active))
    );
}

/// A dead coach key flags the coach's connection and never the member's,
/// and the member's read fails as the coach's, not their own.
async fn a_dead_coach_key_flags_the_coach(w: &World) {
    let repos = &w.res.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&coach_token(&w.coach, DEAD_KEY, API_KEY_TOKEN_TYPE))
        .await
        .unwrap();

    let error = w.head(&w.m1).await.expect_err("the coach's key is dead");
    assert!(is_coach_credential_expired(&error), "{error:?}");
    assert!(
        error.provider_auth_required_provider().is_none(),
        "never the member's reconnect: {error:?}"
    );
    assert_eq!(
        w.connection(&w.coach).await,
        Some((ConnectionType::Manual, ConnectionStatus::NeedsReauth))
    );
    assert_eq!(
        w.connection(&w.m1).await,
        Some((ConnectionType::Delegated, ConnectionStatus::Active))
    );

    // The next read is refused before any call, naming the coach.
    w.api.clear();
    let refused = w.authenticate(&w.m1).await.err().expect("refused");
    assert!(
        refused.contains(&format!("{COACH_NAME} needs to reconnect")),
        "{refused}"
    );
    assert!(w.api.hits().is_empty(), "{:?}", w.api.hits());
    let card = w.intervals_card(&w.m1).await;
    assert_eq!(card["delegation"]["coach_needs_reauth"], true, "{card}");

    // The coach links their key again, and the member's reads resume.
    repos
        .oauth_tokens
        .upsert_token(&coach_token(&w.coach, COACH_KEY, API_KEY_TOKEN_TYPE))
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_active(w.coach.id, w.coach.tenant, INTERVALS_ICU)
        .await
        .unwrap();
    assert_eq!(w.head(&w.m1).await.unwrap(), 1);
}

/// The second athlete stops sharing with the coach: their next read ends
/// the link, and their delegated connection goes with it.
async fn an_athlete_who_stops_sharing_ends_the_link(w: &World, link: Uuid) {
    w.api.drop_second.store(true, Ordering::SeqCst);
    let error = w
        .head(&w.m2)
        .await
        .expect_err("the athlete no longer shares with the coach");
    assert!(is_athlete_off_roster(&error), "{error:?}");

    let ended = w.stored(link).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(ended.revoke_reason, Some(DelegationEndReason::NotOnRoster));
    assert_eq!(w.connection(&w.m2).await, None);
}

/// An Intervals.icu OAuth grant lists no athletes: the roster asks the coach
/// to reconnect with their API key, and a link's read is refused naming the
/// coach.
async fn an_oauth_grant_cannot_read_the_roster(w: &World) {
    let repos = &w.res.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&coach_token(&w.coach, "oauth-token", "Bearer"))
        .await
        .unwrap();
    w.api.clear();
    let (status, body) = w
        .get(
            &format!(
                "{}/roster?provider=intervals_icu&refresh=true",
                w.links_path()
            ),
            &w.coach,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["details"]["reason"], "coach_platform_api_key_required",
        "{body}"
    );
    assert_eq!(body["details"]["provider"], "intervals_icu", "{body}");
    let refused = w.authenticate(&w.m1).await.err().expect("refused");
    assert!(
        refused.contains(&format!("{COACH_NAME} needs to reconnect")),
        "{refused}"
    );
    assert!(w.api.hits().is_empty(), "{:?}", w.api.hits());
}

/// The member is asked, and the coach told, in notices naming the platform
/// the link is on, and the member's notice opens its Intervals.icu card.
async fn the_link_notices_name_intervals_icu(w: &World) {
    let asked = w.notice(&w.m1, "delegation_proposed").await;
    assert_eq!(asked["title"], "Intervals.icu link request", "{asked}");
    assert_eq!(
        asked["body"],
        format!(
            "{COACH_NAME} asked to read your Intervals.icu workouts through their own \
             Intervals.icu account, in Squad. Nothing is read until you confirm."
        ),
        "{asked}"
    );
    assert_eq!(asked["data"]["provider"], INTERVALS_ICU, "{asked}");
    let confirmed = w.notice(&w.coach, "delegation_confirmed").await;
    assert_eq!(
        confirmed["title"], "Intervals.icu link confirmed",
        "{confirmed}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_member_reads_intervals_icu_through_the_coach() {
    let api = Api::default();
    env::set_var(
        "PIERRE_INTERVALS_ICU_API_BASE_URL",
        spawn_api(api.clone()).await,
    );
    let w = world(api).await;

    the_coach_reads_their_intervals_roster(&w).await;
    let m1_link = w.link(M1_ATHLETE, &w.m1).await;
    let m2_link = w.link(M2_ATHLETE, &w.m2).await;
    assert_eq!(w.stored(m1_link).await.provider, INTERVALS_ICU);
    the_link_notices_name_intervals_icu(&w).await;

    the_member_reads_through_the_coachs_key(&w).await;
    a_dead_coach_key_flags_the_coach(&w).await;
    an_athlete_who_stops_sharing_ends_the_link(&w, m2_link).await;
    an_oauth_grant_cannot_read_the_roster(&w).await;

    env::remove_var("PIERRE_INTERVALS_ICU_API_BASE_URL");
}
