// ABOUTME: Every read through a TrainingPeaks coach link re-checks that its roster athlete is the member by email
// ABOUTME: Pins the refusal of a link stored without an email or with another's, the reason on each surface, and the relink
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A link binds a roster athlete to a member by email when it is proposed and
//! confirmed, and every read through it checks that binding again. A link
//! stored before links carried the athlete's email, or one whose email is not
//! the member's verified email, reads nothing: the read is refused in words
//! that say why, before any scrape, and the link is left standing so no stored
//! state flips. The coach's group list, the member's provider card and the
//! connection-status tool carry the reason, which is how the coach sees why.
//! Once the coach proposes the athlete again and the member confirms, the
//! link reads.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide. The scraper
//! is a loopback stand-in (a test double, per the repo's mock rule) that
//! answers the coach's profile and each athlete's workouts, and records the
//! athlete every workout read names.

mod common;
mod helpers;

use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::{SCIOTTE_TRAININGPEAKS, TOKEN_TYPE_SESSION};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionType, CreateAgentRequest, DelegatedConnection, DelegationStatus,
    RosterAthlete, TenantId, UserOAuthToken,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::sciotte_remote::{ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::DelegatedConnectionRoutes;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::implementations::connection::GetConnectionStatusTool;
use pierre_tool_runtime::protocol::auth::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

const PROVIDER: &str = SCIOTTE_TRAININGPEAKS;
/// The coach's live TrainingPeaks session.
const COACH_SESSION: &str = "tp-coach-recheck";

/// The athlete a link stored before links carried an email names: m1.
const M1_ATHLETE: &str = "900001";
/// The athlete a link proposed and confirmed with its email names: m2.
const M2_ATHLETE: &str = "900002";
/// The athlete a link stored with another person's email names: m3.
const M3_ATHLETE: &str = "900003";

const M1_EMAIL: &str = "m1@recheck.test";
const M2_EMAIL: &str = "m2@recheck.test";
const M3_EMAIL: &str = "m3@recheck.test";

/// How long a spawned step (the warm-up after a confirm) gets to land.
const SPAWN_DEADLINE: Duration = Duration::from_secs(15);

/// The athlete each workout read named, in order.
#[derive(Clone, Default)]
struct Scraper {
    reads: Arc<Mutex<Vec<String>>>,
}

impl Scraper {
    fn reads_of(&self, athlete: &str) -> usize {
        self.reads
            .lock()
            .unwrap()
            .iter()
            .filter(|read| *read == athlete)
            .count()
    }
}

/// The coach's own profile: a coach account whose email is the coach's, with
/// each roster athlete listed under the email of the member they are.
async fn athlete_route() -> Json<Value> {
    Json(json!({
        "id": "900101",
        "role": "coach",
        "email": "coach@recheck.test",
        "display_name": "Casey Coach",
        "coached_athletes": [
            { "id": M1_ATHLETE, "display_name": "Alex Athlete", "email": M1_EMAIL },
            { "id": M2_ATHLETE, "display_name": "Sam Swimmer", "email": M2_EMAIL },
            { "id": M3_ATHLETE, "display_name": "Robin Rower", "email": M3_EMAIL }
        ]
    }))
}

async fn activities_route(
    State(scraper): State<Scraper>,
    Query(query): Query<HashMap<String, String>>,
) -> (StatusCode, Json<Value>) {
    let Some(athlete) = query.get("athlete") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "athlete_required" })),
        );
    };
    scraper.reads.lock().unwrap().push(athlete.clone());
    let row = json!({
        "id": format!("{athlete}:5001"),
        "name": "Tempo",
        "sport_type": "run",
        "start_date": (Utc::now() - ChronoDuration::days(1)).to_rfc3339(),
        "duration_seconds": 3_600,
        "provider": "trainingpeaks",
    });
    (
        StatusCode::OK,
        Json(json!({ "count": 1, "activities": [row], "head_complete": true })),
    )
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
        .with_state(scraper);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

/// A user whose email is verified.
async fn person(res: &Arc<ServerContext>, email: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    res.common
        .repos
        .email_verification
        .mark_verified(id)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    Person { id, tenant, auth }
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

async fn world(scraper: Scraper) -> World {
    let res = create_test_server_resources().await.unwrap();
    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@recheck.test").await;
    let coach = person(&res, "coach@recheck.test").await;
    let m1 = person(&res, M1_EMAIL).await;
    let m2 = person(&res, M2_EMAIL).await;
    let m3 = person(&res, M3_EMAIL).await;

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
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: coach.id,
            tenant_id: coach.tenant.to_string(),
            provider: PROVIDER.to_owned(),
            access_token: json!({
                "session_id": COACH_SESSION,
                "cookies": [],
                "created_at": "2026-09-23T12:00:00Z",
                "expires_at": null
            })
            .to_string(),
            refresh_token: None,
            token_type: TOKEN_TYPE_SESSION.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: None,
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
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
        .record_provider_terms(
            coach.id,
            PROVIDER,
            provider_terms_version(PROVIDER).expect("TrainingPeaks carries a notice"),
        )
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

impl World {
    fn links_path(&self) -> String {
        format!("/api/groups/{}/delegated-connections", self.group)
    }

    async fn send(&self, request: AxumTestRequest, who: &Person) -> (StatusCode, Value) {
        let resp = request
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    /// A link confirmed the way links were before they stored the athlete's
    /// email, or stored with `email`: written straight to the repository,
    /// with the member's delegated connection row the confirm registers.
    async fn stored_link(&self, member: &Person, athlete: &str, email: Option<&str>) -> Uuid {
        let repos = &self.res.common.repos;
        let proposed = repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                PROVIDER.to_owned(),
                self.group,
                self.coach.id,
                self.coach.tenant,
                member.id,
                RosterAthlete {
                    id: athlete.to_owned(),
                    name: None,
                    email: email.map(str::to_owned),
                },
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet");
        repos
            .delegated_connections
            .confirm(proposed.id, member.id, member.tenant, Utc::now())
            .await
            .unwrap()
            .expect("the proposal names the member");
        repos
            .provider_connections
            .register_connection(
                member.id,
                member.tenant,
                PROVIDER,
                &ConnectionType::Delegated,
                Some(&json!({ "delegated_connection_id": proposed.id }).to_string()),
            )
            .await
            .unwrap();
        proposed.id
    }

    /// The coach proposes `athlete` as `member` over REST and the member
    /// confirms; the warm-up read the confirm spawns lands. Returns the id.
    async fn link(&self, athlete: &str, member: &Person) -> Uuid {
        let before = self.scraper.reads_of(athlete);
        let body = json!({
            "provider": "trainingpeaks",
            "provider_athlete_id": athlete,
            "member_user_id": member.id,
        });
        let (status, proposed) = self
            .send(
                AxumTestRequest::post(&self.links_path()).json(&body),
                &self.coach,
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{proposed}");
        let id = proposed["id"].as_str().unwrap().to_owned();

        let path = format!("{}/{id}/confirm", self.links_path());
        let (status, confirmed) = self
            .send(AxumTestRequest::post(&path).json(&json!({})), member)
            .await;
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        assert_eq!(confirmed["status"], "confirmed");
        assert_eq!(confirmed["read_refused"], Value::Null, "{confirmed}");

        let deadline = Instant::now() + SPAWN_DEADLINE;
        while self.scraper.reads_of(athlete) == before {
            assert!(Instant::now() < deadline, "the warm-up read never happened");
            sleep(Duration::from_millis(50)).await;
        }
        Uuid::parse_str(&id).unwrap()
    }

    /// The coach ends a live link.
    async fn unlink(&self, link: Uuid) {
        let path = format!("{}/{link}", self.links_path());
        let (status, body) = self.send(AxumTestRequest::delete(&path), &self.coach).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }

    /// The refusal's words when `who`'s TrainingPeaks read is refused.
    async fn refusal(&self, who: &Person) -> Option<String> {
        AuthService::new(Arc::clone(&self.runtime))
            .create_authenticated_provider("trainingpeaks", who.id, Some(&who.tenant.to_string()))
            .await
            .err()
            .map(|response| response.error.unwrap_or_default())
    }

    /// `who`'s recent workouts, read through their link.
    async fn read(&self, who: &Person) -> usize {
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
        .expect("the link serves the read")
        .len()
    }

    /// The link `link` as the coach's group list shows it.
    async fn listed(&self, link: Uuid) -> Value {
        let (status, body) = self
            .send(AxumTestRequest::get(&self.links_path()), &self.coach)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["connections"]
            .as_array()
            .unwrap()
            .iter()
            .find(|listed| listed["id"] == link.to_string())
            .cloned()
            .unwrap_or_else(|| panic!("the coach's list shows {link}: {body}"))
    }

    /// The link `who` reads on their own TrainingPeaks card.
    async fn card_delegation(&self, who: &Person) -> Value {
        let (status, body) = self.send(AxumTestRequest::get("/api/providers"), who).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["provider"] == PROVIDER)
            .map(|card| card["delegation"].clone())
            .expect("the TrainingPeaks card is listed")
    }

    /// The status word `get_connection_status` reports for `who`.
    async fn status_word(&self, who: &Person) -> Value {
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response = GetConnectionStatusTool
            .execute(&self.runtime, &ctx, json!({ "provider": "trainingpeaks" }))
            .await;
        response
            .structured_content
            .clone()
            .unwrap_or_else(|| panic!("the tool answers with structured content: {response:?}"))
            ["status"]
            .clone()
    }

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
}

/// A link confirmed before links stored the athlete's email reads nothing,
/// says why on every surface, and is left standing.
async fn a_link_without_an_email_refuses_the_read(w: &World) -> Uuid {
    let old = w.stored_link(&w.m1, M1_ATHLETE, None).await;

    let refused = w.refusal(&w.m1).await.expect("the read is refused");
    assert!(
        refused.contains("lists no TrainingPeaks email for the roster athlete"),
        "{refused}"
    );
    assert!(
        refused.contains("The coach links the athlete again"),
        "{refused}"
    );
    assert_eq!(
        w.scraper.reads_of(M1_ATHLETE),
        0,
        "refused before any read reached TrainingPeaks"
    );

    assert_eq!(w.listed(old).await["read_refused"], "athlete_email_missing");
    let card = w.card_delegation(&w.m1).await;
    assert_eq!(card["status"], "confirmed", "{card}");
    assert_eq!(card["read_refused"], "athlete_email_missing", "{card}");
    assert_eq!(w.status_word(&w.m1).await, "athlete_email_missing");

    let stored = w.stored(old).await;
    assert_eq!(stored.status, DelegationStatus::Confirmed, "nothing flips");
    assert_eq!(stored.revoke_reason, None);
    old
}

/// A link whose stored email is another person's reads nothing either.
async fn a_link_with_another_email_refuses_the_read(w: &World) {
    let other = w
        .stored_link(&w.m3, M3_ATHLETE, Some("someone-else@recheck.test"))
        .await;

    let refused = w.refusal(&w.m3).await.expect("the read is refused");
    assert!(
        refused.contains("is not this athlete's verified Dravr email"),
        "{refused}"
    );
    assert_eq!(w.scraper.reads_of(M3_ATHLETE), 0);
    assert_eq!(
        w.listed(other).await["read_refused"],
        "athlete_email_mismatch"
    );
    assert_eq!(w.status_word(&w.m3).await, "athlete_email_mismatch");
}

/// A link proposed and confirmed with the member's email reads through the
/// coach, and shows no refusal.
async fn a_matching_link_reads(w: &World) {
    let bound = w.link(M2_ATHLETE, &w.m2).await;
    let before = w.scraper.reads_of(M2_ATHLETE);

    assert_eq!(w.refusal(&w.m2).await, None);
    assert_eq!(w.read(&w.m2).await, 1);
    assert_eq!(w.scraper.reads_of(M2_ATHLETE), before + 1);
    assert_eq!(w.listed(bound).await["read_refused"], Value::Null);
    assert_eq!(w.card_delegation(&w.m2).await["read_refused"], Value::Null);
    assert_eq!(w.status_word(&w.m2).await, "connected");
}

/// Once the coach proposes the athlete again, with the email the roster lists,
/// and the member confirms, the member's workouts read again.
async fn relinking_reads_again(w: &World, old: Uuid) {
    w.unlink(old).await;
    let relinked = w.link(M1_ATHLETE, &w.m1).await;

    assert_eq!(w.refusal(&w.m1).await, None);
    let before = w.scraper.reads_of(M1_ATHLETE);
    assert_eq!(w.read(&w.m1).await, 1);
    assert_eq!(w.scraper.reads_of(M1_ATHLETE), before + 1);
    assert_eq!(w.listed(relinked).await["read_refused"], Value::Null);
    assert_eq!(w.status_word(&w.m1).await, "connected");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_coach_link_reads_only_while_its_athlete_is_the_member_by_email() {
    let scraper = Scraper::default();
    env::set_var(ENV_REMOTE_URL, spawn_scraper(scraper.clone()).await);
    env::remove_var(ENV_AUDIENCE);
    let w = world(scraper).await;

    let old = a_link_without_an_email_refuses_the_read(&w).await;
    a_link_with_another_email_refuses_the_read(&w).await;
    a_matching_link_reads(&w).await;
    relinking_reads_again(&w, old).await;

    env::remove_var(ENV_REMOTE_URL);
}
