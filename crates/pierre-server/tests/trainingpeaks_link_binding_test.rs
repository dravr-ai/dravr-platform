// ABOUTME: A TrainingPeaks coach link binds by email: the coach account to the coach, each roster athlete to the member
// ABOUTME: Pins the roster, propose and confirm refusals when an email is missing, another person's, or unverified
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Coach powers over TrainingPeaks — the roster, and a link reading a
//! member's workouts through the coach's account — hold only while each
//! TrainingPeaks account is the Dravr account it stands for. The coach's
//! TrainingPeaks email must be the coach's verified Dravr email, and a roster
//! athlete's TrainingPeaks email must be the member's verified Dravr email,
//! compared without regard to case; a missing email, another person's, or a
//! Dravr email nobody verified is refused with its reason, which the clients
//! branch on.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide. The scraper
//! is a loopback stand-in (a test double, per the repo's mock rule) that
//! answers the profile read by the coach session it carries.

mod common;
mod helpers;

use std::env;
use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_sciotte::client::{ENV_AUDIENCE, ENV_REMOTE_URL};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    AgentCategory, ConnectionType, CreateAgentRequest, DelegatedConnection, RosterAthlete,
    TenantId, UserOAuthToken,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_groups::DelegatedConnectionRoutes;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

const PROVIDER: &str = oauth_providers::SCIOTTE_TRAININGPEAKS;

/// A coach account whose TrainingPeaks email is the coach's.
const BOUND_COACH_SESSION: &str = "bound-coach";
/// A coach account whose TrainingPeaks email is someone else's.
const STRANGER_COACH_SESSION: &str = "stranger-coach";
/// A coach account TrainingPeaks lists no email for.
const SILENT_COACH_SESSION: &str = "silent-coach";

/// The roster athlete that is m1, listed with m1's email in another case.
const M1_ATHLETE: &str = "900001";
/// A roster athlete listed with an email no member has.
const STRANGER_ATHLETE: &str = "900002";
/// A roster athlete listed with no email.
const SILENT_ATHLETE: &str = "900003";
/// The roster athlete listed with the unverified member's email.
const UNVERIFIED_ATHLETE: &str = "900004";

fn session_json(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "cookies": [],
        "created_at": "2026-09-23T12:00:00Z",
        "expires_at": null
    })
}

fn session_of(headers: &HeaderMap) -> String {
    headers
        .get("x-session-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// The coach profile each session reads: a coach account with the roster,
/// listing the account's email as the session names it.
fn coach_profile(session: &str) -> Value {
    let mut profile = json!({
        "id": "900101",
        "role": "coach",
        "display_name": "Casey Coach",
        "coached_athletes": [
            { "id": M1_ATHLETE, "display_name": "Alex Athlete", "email": "M1@Binding.Test" },
            { "id": STRANGER_ATHLETE, "display_name": "Sam Swimmer",
              "email": "someone-else@binding.test" },
            { "id": SILENT_ATHLETE, "display_name": "Robin Rower" },
            { "id": UNVERIFIED_ATHLETE, "display_name": "Uma Unverified",
              "email": "unverified@binding.test" }
        ]
    });
    match session {
        BOUND_COACH_SESSION => profile["email"] = json!("coach@binding.test"),
        STRANGER_COACH_SESSION => profile["email"] = json!("stranger@binding.test"),
        _ => {}
    }
    profile
}

async fn spawn_scraper() -> String {
    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route(
            "/api/athlete",
            get(|headers: HeaderMap| async move { Json(coach_profile(&session_of(&headers))) }),
        );
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

/// A user whose email is verified when `verified`.
async fn person(res: &Arc<ServerContext>, email: &str, verified: bool) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    if verified {
        res.common
            .repos
            .email_verification
            .mark_verified(id)
            .await
            .unwrap();
    }
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    Person { id, tenant, auth }
}

async fn agent(res: &Arc<ServerContext>, owner: &Person) -> String {
    res.common
        .repos
        .agents
        .create(
            owner.id,
            owner.tenant,
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
        .to_string()
}

/// A group `owner` owns and `coach` coaches, with `members` in it.
async fn group(
    res: &Arc<ServerContext>,
    name: &str,
    owner: &Person,
    coach: &Person,
    members: &[&Person],
) -> Uuid {
    let repos = &res.common.repos;
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id: agent(res, owner).await,
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
    for member in members {
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
    group
}

fn links_path(group: Uuid) -> String {
    format!("/api/groups/{group}/delegated-connections")
}

struct World {
    res: Arc<ServerContext>,
    router: Router,
    group: Uuid,
    coach: Person,
    m1: Person,
    m2: Person,
    m3: Person,
    unverified: Person,
}

impl World {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let owner = person(&res, "owner@binding.test", true).await;
        let coach = person(&res, "coach@binding.test", true).await;
        let m1 = person(&res, "m1@binding.test", true).await;
        let m2 = person(&res, "m2@binding.test", true).await;
        let m3 = person(&res, "m3@binding.test", true).await;
        let unverified = person(&res, "unverified@binding.test", false).await;
        let group = group(&res, "Squad", &owner, &coach, &[&m1, &m2, &m3, &unverified]).await;

        let repos = &res.common.repos;
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
        let router = DelegatedConnectionRoutes::routes(Arc::clone(&res));
        let world = Self {
            res,
            router,
            group,
            coach,
            m1,
            m2,
            m3,
            unverified,
        };
        world.coach_signs_in_as(BOUND_COACH_SESSION).await;
        world
    }

    /// Store `session` as the coach's own TrainingPeaks session.
    async fn coach_signs_in_as(&self, session: &str) {
        let now = Utc::now();
        self.res
            .common
            .repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id: self.coach.id,
                tenant_id: self.coach.tenant.to_string(),
                provider: PROVIDER.to_owned(),
                access_token: session_json(session).to_string(),
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
    }

    async fn send(&self, request: AxumTestRequest, who: &Person) -> (StatusCode, Value) {
        let resp = request
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await;
        let status = resp.status_code();
        (
            status,
            serde_json::from_str(&resp.text()).unwrap_or(Value::Null),
        )
    }

    /// The roster, read live through the coach's current session.
    async fn roster(&self) -> (StatusCode, Value) {
        let path = format!("{}/roster?refresh=true", links_path(self.group));
        self.send(AxumTestRequest::get(&path), &self.coach).await
    }

    async fn propose_in(&self, group: Uuid, athlete: &str, member: &Person) -> (StatusCode, Value) {
        let body = json!({
            "provider": "trainingpeaks",
            "provider_athlete_id": athlete,
            "member_user_id": member.id,
        });
        self.send(
            AxumTestRequest::post(&links_path(group)).json(&body),
            &self.coach,
        )
        .await
    }

    async fn propose(&self, athlete: &str, member: &Person) -> (StatusCode, Value) {
        self.propose_in(self.group, athlete, member).await
    }

    async fn confirm(&self, link: &str, who: &Person) -> (StatusCode, Value) {
        let path = format!("{}/{link}/confirm", links_path(self.group));
        self.send(AxumTestRequest::post(&path).json(&json!({})), who)
            .await
    }

    async fn manages_roster(&self) -> bool {
        self.res
            .common
            .repos
            .users
            .get_global(self.coach.id)
            .await
            .unwrap()
            .unwrap()
            .manages_roster
    }

    /// A proposal stored as the repository keeps it, naming `email` as the
    /// athlete's roster email.
    async fn stored_proposal(&self, member: &Person, athlete: &str, email: Option<&str>) -> String {
        self.res
            .common
            .repos
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
            .expect("no live link holds this member or athlete yet")
            .id
            .to_string()
    }
}

/// Assert a refusal with `status` whose `details.reason` is `reason`.
fn assert_refused((status, body): (StatusCode, Value), expected: StatusCode, reason: &str) {
    assert_eq!(status, expected, "{body}");
    assert_eq!(body["details"]["reason"], reason, "{body}");
}

/// The coach's own coach account reads the roster and earns the grant;
/// another person's, or one with no email, is refused and takes it back.
async fn the_roster_binds_the_coach_account_by_email(w: &World) {
    let (status, body) = w.roster().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["athletes"].as_array().unwrap().len(), 4);
    assert!(w.manages_roster().await, "the coach's own coach account");

    w.coach_signs_in_as(STRANGER_COACH_SESSION).await;
    assert_refused(
        w.roster().await,
        StatusCode::BAD_REQUEST,
        "coach_platform_email_mismatch",
    );
    assert!(
        !w.manages_roster().await,
        "someone else's coach account takes the grant back"
    );

    w.coach_signs_in_as(SILENT_COACH_SESSION).await;
    assert_refused(
        w.roster().await,
        StatusCode::BAD_REQUEST,
        "coach_platform_email_missing",
    );

    w.coach_signs_in_as(BOUND_COACH_SESSION).await;
    let (status, body) = w.roster().await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// A proposal names only a roster athlete whose email is the member's
/// verified email, case aside, and stores that email for the confirm.
async fn a_proposal_binds_the_athlete_to_the_member(w: &World) -> String {
    let bad = StatusCode::BAD_REQUEST;
    assert_refused(
        w.propose(STRANGER_ATHLETE, &w.m2).await,
        bad,
        "athlete_email_mismatch",
    );
    assert_refused(
        w.propose(SILENT_ATHLETE, &w.m2).await,
        bad,
        "athlete_email_missing",
    );
    assert_refused(
        w.propose(UNVERIFIED_ATHLETE, &w.unverified).await,
        bad,
        "member_email_unverified",
    );

    let (status, link) = w.propose(M1_ATHLETE, &w.m1).await;
    assert_eq!(status, StatusCode::CREATED, "{link}");
    let id = link["id"].as_str().unwrap().to_owned();
    let stored = w
        .res
        .common
        .repos
        .delegated_connections
        .get_for_participant(Uuid::parse_str(&id).unwrap(), w.group, w.coach.id)
        .await
        .unwrap()
        .unwrap();
    // The roster lists `M1@Binding.Test`; it is stored in its normalized form.
    assert_eq!(
        stored.provider_athlete_email.as_deref(),
        Some("m1@binding.test")
    );
    id
}

/// The member confirms a link whose athlete is them by email; a link whose
/// roster email is missing, another person's, or unproven on the member's
/// side is refused.
async fn a_confirm_checks_the_binding_again(w: &World, bound: &str) {
    let (status, body) = w.confirm(bound, &w.m1).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "confirmed");

    let bad = StatusCode::BAD_REQUEST;
    let carries_none = w.stored_proposal(&w.m2, SILENT_ATHLETE, None).await;
    assert_refused(
        w.confirm(&carries_none, &w.m2).await,
        bad,
        "athlete_email_missing",
    );
    let someone_else = w
        .stored_proposal(&w.m3, STRANGER_ATHLETE, Some("someone-else@binding.test"))
        .await;
    assert_refused(
        w.confirm(&someone_else, &w.m3).await,
        bad,
        "athlete_email_mismatch",
    );
    let unproven = w
        .stored_proposal(
            &w.unverified,
            UNVERIFIED_ATHLETE,
            Some("unverified@binding.test"),
        )
        .await;
    assert_refused(
        w.confirm(&unproven, &w.unverified).await,
        bad,
        "dravr_email_unverified",
    );
}

/// A bound athlete already linked through another group the coach coaches
/// is refused as linked, after the binding holds.
async fn a_bound_athlete_is_linked_once(w: &World) {
    let owner = person(&w.res, "second-owner@binding.test", true).await;
    let second = group(&w.res, "Second squad", &owner, &w.coach, &[&w.m1]).await;
    assert_refused(
        w.propose_in(second, M1_ATHLETE, &w.m1).await,
        StatusCode::CONFLICT,
        "athlete_already_linked",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trainingpeaks_link_binds_each_account_to_its_dravr_account_by_email() {
    env::set_var(ENV_REMOTE_URL, spawn_scraper().await);
    env::remove_var(ENV_AUDIENCE);
    let w = World::new().await;

    the_roster_binds_the_coach_account_by_email(&w).await;
    let bound = a_proposal_binds_the_athlete_to_the_member(&w).await;
    a_confirm_checks_the_binding_again(&w, &bound).await;
    a_bound_athlete_is_linked_once(&w).await;

    env::remove_var(ENV_REMOTE_URL);
}
