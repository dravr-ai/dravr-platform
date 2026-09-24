// ABOUTME: A group's coach links TrainingPeaks roster athletes to members over REST; members confirm, decline or unlink
// ABOUTME: Pins who may take each step, the refusal reasons, the rows each step writes and the feed row each side reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A TrainingPeaks coach account has no calendar of its own; its session
//! reads the athletes on its roster. A group's human coach links a roster
//! athlete to a live member (`POST …/delegated-connections`), the member
//! confirms (`…/confirm`) — their consent to the read — and either side
//! ends it (`DELETE`). The member is told when asked; the coach is told the
//! answer; both are told when TrainingPeaks drops the athlete from the
//! coach's roster. Every notice is read back from the feed the clients read,
//! in the reader's own language.
//!
//! One test, because `DRAVR_SCIOTTE_REMOTE_URL` is process-wide: separate
//! tests in this binary would race on the scraper they point at. The scraper
//! is a loopback stand-in (a test double, per the repo's mock rule) that
//! answers by session and counts the roster reads it serves.

mod common;
mod helpers;

use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::{
    self as oauth_providers, TRAININGPEAKS_TERMS_VERSION,
};
use pierre_core::constants::oauth_providers::TOKEN_TYPE_SESSION;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole, UpdateGroupRequest,
};
use pierre_core::models::{
    AgentCategory, ConnectionType, CreateAgentRequest, DelegatedConnection, DelegationEndReason,
    DelegationStatus, ProviderAccountRole, TenantId, UserOAuthToken,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_providers::sciotte_remote::{CoachedAthlete, ENV_AUDIENCE, ENV_REMOTE_URL};
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::{DelegatedConnectionRoutes, GroupRoutes, NotificationRoutes};
use pierre_services::delegated_connections::{end_off_roster, roster_cache_key};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

const PROVIDER: &str = oauth_providers::SCIOTTE_TRAININGPEAKS;
const COACH_SESSION: &str = "coach-session";
const ATHLETE_SESSION: &str = "athlete-session";
const GROUP_NAME: &str = "Squad";
const COACH_NAME: &str = "Casey Coach";
const M1_NAME: &str = "Alex Athléte";
const M2_NAME: &str = "Sam Swimmer";

/// How long a fire-and-forget notice gets to land in the feed.
const NOTICE_DEADLINE: Duration = Duration::from_secs(10);

/// Roster reads the stand-in served.
type Calls = Arc<Mutex<usize>>;

/// Athlete ids the stand-in's roster no longer lists.
type Dropped = Arc<Mutex<Vec<&'static str>>>;

/// The coach's roster as `TrainingPeaks` lists it, before any drop.
const ROSTER: [(&str, Option<&str>); 3] = [
    ("900001", Some("Alex Athlete")),
    ("900002", Some("Sam Swimmer")),
    ("900003", None),
];

fn session_json(session_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "cookies": [],
        "created_at": "2026-09-23T12:00:00Z",
        "expires_at": null
    })
}

/// The scraper stand-in: a coach session reads the three-athlete roster,
/// less the athletes dropped from it; any other session is an athlete
/// account. A login's email picks the account.
async fn spawn_scraper(roster_reads: Calls, dropped: Dropped) -> String {
    let app = Router::new()
        .route(
            "/auth/login-with-credentials",
            post(|Json(body): Json<Value>| async move {
                let session = if body["email"]
                    .as_str()
                    .unwrap_or_default()
                    .starts_with("coach")
                {
                    COACH_SESSION
                } else {
                    ATHLETE_SESSION
                };
                Json(json!({
                    "status": "authenticated",
                    "session_id": session,
                    "provider": body["provider"],
                }))
            }),
        )
        .route(
            "/auth/sessions/{session_id}/export",
            get(|Path(session_id): Path<String>| async move {
                Json(json!({ "session": session_json(&session_id) }))
            }),
        )
        .route(
            "/auth/import-session",
            post(|Json(body): Json<Value>| async move {
                Json(json!({ "session_id": body["session"]["session_id"] }))
            }),
        )
        .route(
            "/api/athlete",
            get(
                |State((reads, dropped)): State<(Calls, Dropped)>, headers: HeaderMap| async move {
                    let session = headers
                        .get("x-session-id")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    if session == COACH_SESSION {
                        *reads.lock().unwrap() += 1;
                        let dropped = dropped.lock().unwrap().clone();
                        let athletes: Vec<Value> = ROSTER
                            .iter()
                            .filter(|(id, _)| !dropped.contains(id))
                            .map(|(id, name)| {
                                let mut athlete = json!({ "id": id });
                                if let Some(name) = name {
                                    athlete["display_name"] = json!(name);
                                }
                                athlete
                            })
                            .collect();
                        return Json(json!({
                            "id": "900101",
                            "role": "coach",
                            "coached_athletes": athletes,
                            "display_name": "Casey Coach"
                        }));
                    }
                    Json(json!({ "id": "900001", "role": "athlete", "display_name": "Alex" }))
                },
            ),
        )
        .route(
            "/api/activities",
            get(|| async { Json(json!({ "activities": [], "head_complete": true })) }),
        )
        .with_state((roster_reads, dropped));
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
    email: String,
}

struct World {
    res: Arc<ServerContext>,
    router: Router,
    group: Uuid,
    coach: Person,
    m1: Person,
    m2: Person,
    outsider: Person,
    roster_reads: Calls,
    dropped: Dropped,
}

async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    let users = &res.common.repos.users;
    users.update_display_name(id, name).await.unwrap();
    users.update_locale(id, "en").await.unwrap();
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    Person {
        id,
        tenant,
        auth,
        email: email.to_owned(),
    }
}

async fn world(roster_reads: Calls, dropped: Dropped) -> World {
    let res = create_test_server_resources().await.unwrap();
    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@links.test", "Olive Owner").await;
    let coach = person(&res, "coach@links.test", COACH_NAME).await;
    let m1 = person(&res, "m1@links.test", M1_NAME).await;
    let m2 = person(&res, "m2@links.test", M2_NAME).await;
    let outsider = person(&res, "outsider@links.test", "Otto Outsider").await;

    let agent_id = repos
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
        .to_string();
    let now = Utc::now();
    let group = repos
        .groups
        .create_group(
            owner.tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: owner.tenant.to_string(),
                name: GROUP_NAME.to_owned(),
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

    let router = GroupRoutes::routes(Arc::clone(&res))
        .merge(DelegatedConnectionRoutes::routes(Arc::clone(&res)))
        .merge(NotificationRoutes::routes(Arc::clone(&res)));
    World {
        res,
        router,
        group,
        coach,
        m1,
        m2,
        outsider,
        roster_reads,
        dropped,
    }
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

impl World {
    fn links_path(&self) -> String {
        format!("/api/groups/{}/delegated-connections", self.group)
    }

    /// The account role recorded on the coach's own TrainingPeaks connection.
    async fn recorded_role(&self) -> Option<ProviderAccountRole> {
        self.res
            .common
            .repos
            .provider_connections
            .get_for_user(self.coach.id, Some(self.coach.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == PROVIDER)
            .and_then(|c| c.account_role)
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

    async fn delete(&self, path: &str, who: &Person) -> (StatusCode, Value) {
        let resp = AxumTestRequest::delete(path)
            .header("authorization", &who.auth)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    async fn propose(&self, athlete: &str, member: Uuid) -> (StatusCode, Value) {
        self.post(
            &self.links_path(),
            &self.coach,
            &json!({
                "provider": "trainingpeaks",
                "provider_athlete_id": athlete,
                "member_user_id": member,
            }),
        )
        .await
    }

    async fn confirm(&self, link: &str, who: &Person) -> (StatusCode, Value) {
        self.post(
            &format!("{}/{link}/confirm", self.links_path()),
            who,
            &json!({}),
        )
        .await
    }

    async fn end(&self, link: &str, who: &Person) -> (StatusCode, Value) {
        self.delete(&format!("{}/{link}", self.links_path()), who)
            .await
    }

    /// The link as stored, read as its coach.
    async fn stored(&self, link: &str) -> DelegatedConnection {
        self.res
            .common
            .repos
            .delegated_connections
            .get_for_participant(Uuid::parse_str(link).unwrap(), self.group, self.coach.id)
            .await
            .unwrap()
            .expect("an ended link stays for audit")
    }

    async fn connection_type(&self, who: &Person) -> Option<ConnectionType> {
        self.res
            .common
            .repos
            .provider_connections
            .get_for_user(who.id, Some(who.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == PROVIDER)
            .map(|c| c.connection_type)
    }

    fn roster_reads(&self) -> usize {
        *self.roster_reads.lock().unwrap()
    }

    /// The feed row of `wire` type `who` reads, waiting for the
    /// fire-and-forget dispatch to land.
    async fn notice(&self, who: &Person, wire: &str) -> Value {
        self.notices(who, wire, 1).await.swap_remove(0)
    }

    /// The feed rows of `wire` type `who` reads, waiting until `count` of
    /// them have landed.
    async fn notices(&self, who: &Person, wire: &str, count: usize) -> Vec<Value> {
        let started = Instant::now();
        loop {
            let (status, body) = self.get("/api/notifications", who).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let rows: Vec<Value> = body["data"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|row| row["notification_type"] == wire)
                .cloned()
                .collect();
            if rows.len() >= count {
                return rows;
            }
            assert!(
                started.elapsed() < NOTICE_DEADLINE,
                "{} of {count} {wire} notices reached {}",
                rows.len(),
                who.email
            );
            sleep(Duration::from_millis(100)).await;
        }
    }

    /// The athlete ids the roster at `path` lists, as the coach reads it.
    async fn roster_ids(&self, path: &str) -> Vec<String> {
        let (status, body) = self.get(path, &self.coach).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["provider_athlete_id"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// Assert a refusal with `status` whose `details.reason` is `reason`.
fn assert_refused((status, body): (StatusCode, Value), expected: StatusCode, reason: &str) {
    assert_eq!(status, expected, "{body}");
    assert_eq!(body["details"]["reason"], reason, "{body}");
}

/// Assert a 403 carrying exactly `message`.
fn assert_denied((status, body): (StatusCode, Value), message: &str) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["message"], message);
}

/// Assert a feed row's category, title and body.
fn assert_notice(row: &Value, title: &str, body: &str) {
    assert_eq!(row["category"], "coach");
    assert_eq!(row["title"], title);
    assert_eq!(row["body"], body);
}

const COACH_ONLY: &str = "Only the group's coach can link TrainingPeaks athletes";

/// The coach holds no membership, yet reads the group's info and members;
/// every other group surface keeps its member gate.
async fn the_coach_sees_the_group(w: &World) {
    let group_path = format!("/api/groups/{}", w.group);
    let (status, body) = w.get(&group_path, &w.coach).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["name"], GROUP_NAME);

    let (status, body) = w.get(&format!("{group_path}/members"), &w.coach).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 2);

    let (status, _) = w.get(&format!("{group_path}/invites"), &w.coach).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "invites keep the member gate"
    );

    let (status, _) = w.get(&group_path, &w.outsider).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

fn roster_path(w: &World) -> String {
    format!("{}/roster", w.links_path())
}

/// Only the coach reads the roster, and only with the current notice
/// accepted; the refusal comes before any scrape.
async fn only_the_coach_reads_the_roster_with_the_notice_accepted(w: &World) {
    assert_denied(w.get(&roster_path(w), &w.m1).await, COACH_ONLY);

    let users = &w.res.common.repos.users;
    users
        .record_trainingpeaks_terms(w.coach.id, "2026-01-01")
        .await
        .unwrap();
    assert_refused(
        w.get(&roster_path(w), &w.coach).await,
        StatusCode::BAD_REQUEST,
        "trainingpeaks_terms_outdated",
    );
    assert_eq!(w.roster_reads(), 0, "refused before any scrape");
    users
        .record_trainingpeaks_terms(w.coach.id, TRAININGPEAKS_TERMS_VERSION)
        .await
        .unwrap();
}

/// A roster planted under the coach's key before their connection's role was
/// ever read is not served: the first read goes live, which is what records
/// the account as a coach's.
async fn an_unread_role_never_serves_a_cached_roster(w: &World) {
    assert_eq!(w.recorded_role().await, None);
    let planted = vec![CoachedAthlete {
        id: "999999".to_owned(),
        display_name: Some("Someone Else".to_owned()),
    }];
    w.res
        .common
        .cache
        .set(
            &roster_cache_key(w.coach.id, w.coach.tenant),
            &planted,
            Duration::from_mins(10),
        )
        .await
        .unwrap();

    let (status, body) = w.get(&roster_path(w), &w.coach).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ids: Vec<&str> = body["athletes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["provider_athlete_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["900001", "900002", "900003"]);
    assert_eq!(w.roster_reads(), 1, "read live");
    assert_eq!(w.recorded_role().await, Some(ProviderAccountRole::Coach));
}

/// The roster lists every athlete, and suggests the member whose name reads
/// as the athlete's.
async fn the_roster_suggests_members_by_name(w: &World) {
    let (status, body) = w.get(&roster_path(w), &w.coach).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["provider"], "trainingpeaks");
    let athletes = body["athletes"].as_array().unwrap();
    assert_eq!(athletes.len(), 3);
    assert_eq!(athletes[0]["provider_athlete_id"], "900001");
    assert_eq!(athletes[0]["display_name"], "Alex Athlete");
    assert_eq!(athletes[0]["connection"], Value::Null);
    // "Alex Athlete" reads as the member "Alex Athléte": case and accents aside.
    assert_eq!(athletes[0]["suggested_member_user_id"], w.m1.id.to_string());
    assert_eq!(athletes[1]["suggested_member_user_id"], w.m2.id.to_string());
    assert_eq!(athletes[2]["display_name"], Value::Null);
    assert_eq!(athletes[2]["suggested_member_user_id"], Value::Null);
}

/// The read is the fourth way a coach account is learned and granted, and
/// it is cached until asked to refresh.
async fn the_roster_read_grants_the_coach_and_is_cached(w: &World) {
    let repos = &w.res.common.repos;
    let role = repos
        .provider_connections
        .get_for_user(w.coach.id, Some(w.coach.tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == PROVIDER)
        .and_then(|c| c.account_role);
    assert_eq!(role, Some(ProviderAccountRole::Coach));
    let coach = repos.users.get_global(w.coach.id).await.unwrap().unwrap();
    assert!(coach.manages_roster);

    assert_eq!(w.roster_reads(), 1);
    let (status, _) = w.get(&roster_path(w), &w.coach).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(w.roster_reads(), 1, "the second read is the cached roster");

    // The role is recorded once, when it is learned: a live read of an
    // account already recorded as a coach's writes nothing, so it does not
    // re-grant a permission an operator took back since.
    repos
        .users
        .set_manages_roster(w.coach.id, false)
        .await
        .unwrap();
    let (status, _) = w
        .get(&format!("{}?refresh=true", roster_path(w)), &w.coach)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(w.roster_reads(), 2, "refresh reads it live");
    let coach = repos.users.get_global(w.coach.id).await.unwrap().unwrap();
    assert!(
        !coach.manages_roster,
        "an unchanged role is not re-recorded"
    );
    repos
        .users
        .set_manages_roster(w.coach.id, true)
        .await
        .unwrap();
}

/// A new TrainingPeaks login drops the roster read through the session it
/// replaces: the login may be another account's.
async fn a_new_coach_login_drops_the_cached_roster(w: &World) {
    let key = roster_cache_key(w.coach.id, w.coach.tenant);
    let cache = &w.res.common.cache;
    assert!(cache
        .get::<Vec<CoachedAthlete>>(&key)
        .await
        .unwrap()
        .is_some());
    // The stored session lapsed, so the login signs in afresh rather than
    // reusing it.
    let mut lapsed = session_token(w.coach.id, w.coach.tenant, COACH_SESSION);
    lapsed.access_token = json!({
        "session_id": COACH_SESSION,
        "cookies": [],
        "created_at": "2026-09-23T12:00:00Z",
        "expires_at": "2026-09-23T13:00:00Z"
    })
    .to_string();
    w.res
        .common
        .repos
        .oauth_tokens
        .upsert_token(&lapsed)
        .await
        .unwrap();
    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &w.coach.auth)
        .json(&json!({
            "email": "coach@links.test",
            "password": "not-a-real-password",
            "target": "trainingpeaks",
            "tos_consent": true,
        }))
        .send(AuthRoutes::routes(w.res.auth_routes_context()))
        .await;
    let status = resp.status_code();
    let text = resp.text();
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(cache
        .get::<Vec<CoachedAthlete>>(&key)
        .await
        .unwrap()
        .is_none());
}

/// Every proposal rule refuses with its reason.
async fn every_proposal_rule_refuses_with_its_reason(w: &World) {
    let bad = StatusCode::BAD_REQUEST;
    assert_refused(
        w.propose("999", w.m1.id).await,
        bad,
        "athlete_not_on_roster",
    );
    assert_refused(
        w.propose("not an id!", w.m1.id).await,
        bad,
        "invalid_athlete",
    );
    assert_refused(
        w.propose("900001", w.coach.id).await,
        bad,
        "member_is_coach",
    );
    let (status, _) = w.propose("900001", w.outsider.id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "not a live member");
    let strava = json!({
        "provider": "strava",
        "provider_athlete_id": "900001",
        "member_user_id": w.m1.id,
    });
    assert_refused(
        w.post(&w.links_path(), &w.coach, &strava).await,
        bad,
        "unsupported_provider",
    );
    let by_member = json!({
        "provider": "trainingpeaks",
        "provider_athlete_id": "900001",
        "member_user_id": w.m1.id,
    });
    assert_denied(w.post(&w.links_path(), &w.m1, &by_member).await, COACH_ONLY);
}

/// The coach proposes; a second live link for the member or the athlete is
/// refused.
async fn the_coach_proposes(w: &World) -> String {
    let (status, link) = w.propose("900001", w.m1.id).await;
    assert_eq!(status, StatusCode::CREATED, "{link}");
    assert_eq!(link["status"], "proposed");
    assert_eq!(link["provider"], "trainingpeaks");
    assert_eq!(link["coach_display_name"], COACH_NAME);
    assert_eq!(link["member_user_id"], w.m1.id.to_string());
    assert_eq!(link["member_display_name"], w.m1.email);
    assert_eq!(link["provider_athlete_id"], "900001");
    assert_eq!(link["provider_athlete_name"], "Alex Athlete");
    assert_eq!(link["confirmed_at"], Value::Null);

    let conflict = StatusCode::CONFLICT;
    assert_refused(
        w.propose("900002", w.m1.id).await,
        conflict,
        "already_proposed",
    );
    assert_refused(
        w.propose("900001", w.m2.id).await,
        conflict,
        "athlete_already_linked",
    );
    link["id"].as_str().unwrap().to_owned()
}

/// The member reads the request in their feed, in their language, and it
/// opens their connections.
async fn the_member_is_asked(w: &World) {
    let notice = w.notice(&w.m1, "delegation_proposed").await;
    assert_notice(
        &notice,
        "TrainingPeaks link request",
        "Casey Coach asked to read your TrainingPeaks workouts through their own \
         TrainingPeaks account, in Squad. Nothing is read until you confirm.",
    );
    assert_eq!(notice["data"]["screen"], "connections");
    assert_eq!(notice["data"]["provider"], "trainingpeaks");
    assert_eq!(notice["data"]["params"]["coach_name"], COACH_NAME);
    assert_eq!(notice["data"]["params"]["group_name"], GROUP_NAME);
}

/// The coach lists every live link, a member only their own.
async fn each_side_lists_what_it_may_see(w: &World, link: &str) {
    let (status, body) = w.get(&w.links_path(), &w.coach).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["viewer"], "coach");
    assert_eq!(body["total"], 1);
    let (_, body) = w.get(&w.links_path(), &w.m1).await;
    assert_eq!(body["viewer"], "member");
    assert_eq!(body["connections"][0]["id"], link);
    let (_, body) = w.get(&w.links_path(), &w.m2).await;
    assert_eq!(body["viewer"], "member");
    assert_eq!(body["total"], 0);
    let (status, _) = w.get(&w.links_path(), &w.outsider).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Only the member a link names confirms it, and not over a TrainingPeaks
/// login of their own.
async fn only_the_named_member_confirms_without_a_login_of_their_own(w: &World, link: &str) {
    const MEMBER_ONLY: &str = "Only the member a link names can confirm it";
    assert_denied(w.confirm(link, &w.coach).await, MEMBER_ONLY);
    assert_denied(w.confirm(link, &w.m2).await, MEMBER_ONLY);

    let tokens = &w.res.common.repos.oauth_tokens;
    tokens
        .upsert_token(&session_token(w.m1.id, w.m1.tenant, ATHLETE_SESSION))
        .await
        .unwrap();
    assert_refused(
        w.confirm(link, &w.m1).await,
        StatusCode::CONFLICT,
        "own_connection",
    );
    tokens
        .delete_token(w.m1.id, w.m1.tenant, PROVIDER)
        .await
        .unwrap();
}

/// The confirm registers the member's delegated connection in the tenant
/// they confirmed in, and tells the coach.
async fn the_confirm_registers_the_link_and_tells_the_coach(w: &World, link: &str) {
    let (status, body) = w.confirm(link, &w.m1).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "confirmed");
    assert!(body["confirmed_at"].is_string());
    assert_eq!(
        w.connection_type(&w.m1).await,
        Some(ConnectionType::Delegated)
    );
    assert_eq!(w.stored(link).await.member_tenant_id, Some(w.m1.tenant));
    let (status, _) = w.confirm(link, &w.m1).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "no longer a proposal");

    let notice = w.notice(&w.coach, "delegation_confirmed").await;
    assert_notice(
        &notice,
        "TrainingPeaks link confirmed",
        "Alex Athléte confirmed your TrainingPeaks link in Squad. Their workouts are \
         now read through your account.",
    );
    assert_eq!(notice["data"]["screen"], Value::Null);
    assert_eq!(notice["data"]["params"]["member_name"], M1_NAME);
}

/// The member's own TrainingPeaks login takes the link's place.
async fn an_own_login_supersedes_the_link(w: &World, link: &str) {
    let resp = AxumTestRequest::post("/api/providers/sciotte/login")
        .header("authorization", &w.m1.auth)
        .json(&json!({
            "email": "m1@links.test",
            "password": "not-a-real-password",
            "target": "trainingpeaks",
            "tos_consent": true,
        }))
        .send(AuthRoutes::routes(w.res.auth_routes_context()))
        .await;
    let status = resp.status_code();
    let text = resp.text();
    assert_eq!(status, StatusCode::OK, "{text}");

    let ended = w.stored(link).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(ended.revoke_reason, Some(DelegationEndReason::Superseded));
    assert_eq!(ended.revoked_by, Some(w.m1.id));
    assert_eq!(
        w.connection_type(&w.m1).await,
        Some(ConnectionType::Manual),
        "the member's own login, not the coach's link"
    );
}

async fn proposed_to_m2(w: &World) -> String {
    let (status, link) = w.propose("900002", w.m2.id).await;
    assert_eq!(status, StatusCode::CREATED, "{link}");
    link["id"].as_str().unwrap().to_owned()
}

async fn confirmed_by_m2(w: &World) -> String {
    let link = proposed_to_m2(w).await;
    let (status, body) = w.confirm(&link, &w.m2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        w.connection_type(&w.m2).await,
        Some(ConnectionType::Delegated)
    );
    link
}

/// A member declines a proposal and the coach is told; nobody else can end it.
async fn a_member_declines_and_the_coach_is_told(w: &World) {
    let declined = proposed_to_m2(w).await;
    let (status, _) = w.end(&declined, &w.outsider).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = w.end(&declined, &w.m2).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let stored = w.stored(&declined).await;
    assert_eq!(stored.revoke_reason, Some(DelegationEndReason::Declined));
    assert_eq!(stored.revoked_by, Some(w.m2.id));

    let notice = w.notice(&w.coach, "delegation_declined").await;
    assert_notice(
        &notice,
        "TrainingPeaks link declined",
        "Sam Swimmer declined your TrainingPeaks link in Squad.",
    );
}

/// The coach withdraws a proposal, and revokes a confirmed link, whose
/// delegated connection goes with it.
async fn the_coach_withdraws_and_revokes(w: &World) {
    let withdrawn = proposed_to_m2(w).await;
    let (status, _) = w.end(&withdrawn, &w.coach).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        w.stored(&withdrawn).await.revoke_reason,
        Some(DelegationEndReason::Withdrawn)
    );

    let revoked = confirmed_by_m2(w).await;
    assert_denied(
        w.end(&revoked, &w.outsider).await,
        "Only the group's coach or the linked member can end this link",
    );
    let (status, _) = w.end(&revoked, &w.coach).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let stored = w.stored(&revoked).await;
    assert_eq!(
        stored.revoke_reason,
        Some(DelegationEndReason::RevokedByCoach)
    );
    assert!(stored.confirmed_at.is_some());
    assert_eq!(w.connection_type(&w.m2).await, None);
    let (status, _) = w.end(&revoked, &w.coach).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "already ended");
}

const OFF_ROSTER_COACH_BODY: &str = "Your TrainingPeaks roster no longer lists Sam Swimmer, so \
     their link in Squad ended. Link them again once they are back on your roster.";
const OFF_ROSTER_MEMBER_BODY: &str = "Casey Coach's TrainingPeaks roster no longer lists you, \
     so your workouts are no longer read through their account in Squad. Ask Casey Coach to \
     link you again, or connect TrainingPeaks yourself.";

/// TrainingPeaks no longer lists the athlete on the coach's roster: the link
/// ends as the system's doing, and both sides are told.
async fn a_roster_drop_ends_the_link_and_tells_both_sides(w: &World) {
    let link = confirmed_by_m2(w).await;
    let ended = end_off_roster(
        &w.res.common.repos,
        w.res.common.notification_service.as_ref(),
        w.m2.id,
        w.m2.tenant,
        PROVIDER,
    )
    .await
    .unwrap();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].id.to_string(), link);
    assert_eq!(
        ended[0].revoke_reason,
        Some(DelegationEndReason::NotOnRoster)
    );
    assert_eq!(ended[0].revoked_by, None);
    assert_eq!(w.connection_type(&w.m2).await, None);

    let coach_notice = w.notice(&w.coach, "delegation_off_roster").await;
    assert_notice(
        &coach_notice,
        "TrainingPeaks link ended",
        OFF_ROSTER_COACH_BODY,
    );
    let member_notice = w.notice(&w.m2, "delegation_off_coach_roster").await;
    assert_notice(
        &member_notice,
        "TrainingPeaks link ended",
        OFF_ROSTER_MEMBER_BODY,
    );
    assert_eq!(member_notice["data"]["screen"], "connections");
}

/// A proposal whose athlete left the roster ends on the next live roster
/// read, before the roster is answered: the coach could no longer see it to
/// withdraw it, while it kept the member from being linked. A roster served
/// from the cache ends nothing.
async fn a_live_roster_read_ends_a_proposal_off_the_roster(w: &World) {
    let link = proposed_to_m2(w).await;
    let refresh = format!("{}?refresh=true", roster_path(w));
    assert_eq!(w.roster_ids(&refresh).await, ["900001", "900002", "900003"]);
    w.dropped.lock().unwrap().push("900002");

    let reads = w.roster_reads();
    assert_eq!(
        w.roster_ids(&roster_path(w)).await,
        ["900001", "900002", "900003"],
        "the cached roster, read before the drop"
    );
    assert_eq!(w.roster_reads(), reads, "served from the cache");
    assert_eq!(w.stored(&link).await.status, DelegationStatus::Proposed);

    assert_eq!(w.roster_ids(&refresh).await, ["900001", "900003"]);
    assert_eq!(w.roster_reads(), reads + 1, "read live");
    let ended = w.stored(&link).await;
    assert_eq!(ended.status, DelegationStatus::Revoked);
    assert_eq!(ended.revoke_reason, Some(DelegationEndReason::NotOnRoster));
    assert_eq!(ended.revoked_by, None);

    let (status, body) = w.get(&w.links_path(), &w.m2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["total"], 0, "the member's request is gone: {body}");
    let (status, body) = w.propose("900003", w.m2.id).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // The confirmed link the previous step ended told each side once; this
    // proposal tells each side again.
    for row in w.notices(&w.coach, "delegation_off_roster", 2).await {
        assert_notice(&row, "TrainingPeaks link ended", OFF_ROSTER_COACH_BODY);
    }
    for row in w.notices(&w.m2, "delegation_off_coach_roster", 2).await {
        assert_notice(&row, "TrainingPeaks link ended", OFF_ROSTER_MEMBER_BODY);
    }
}

/// An inactive group admits nobody, member or coach, on the group surfaces
/// and in its room alike: only an active group is listed, and both gates ask
/// the one rule.
async fn an_inactive_group_admits_nobody(w: &World) {
    let group_path = format!("/api/groups/{}", w.group);
    let transcript_path = format!("/api/chat/groups/{}/transcript", w.group);
    let chat = ChatRoutes::routes(Arc::clone(&w.res));
    let transcript = |who: &Person| {
        AxumTestRequest::get(&transcript_path)
            .header("authorization", &who.auth)
            .send(chat.clone())
    };
    assert_eq!(w.get(&group_path, &w.m1).await.0, StatusCode::OK);
    assert_eq!(transcript(&w.coach).await.status_code(), StatusCode::OK);

    // The flag alone, so the members' rows stay live.
    let groups = &w.res.common.repos.groups;
    let owner_tenant = groups
        .get_group(&w.group.to_string(), w.m1.tenant)
        .await
        .unwrap()
        .unwrap()
        .tenant_id;
    let deactivated = groups
        .update_group(
            &w.group.to_string(),
            TenantId::parse_str(&owner_tenant).unwrap(),
            &UpdateGroupRequest {
                name: None,
                description: None,
                agent_id: None,
                max_members: None,
                peer_data_sharing: None,
                respond_mode: None,
                digest_mode: None,
                is_active: Some(false),
            },
        )
        .await
        .unwrap()
        .expect("the owner's tenant holds the group");
    assert!(!deactivated.is_active);
    for who in [&w.m1, &w.coach] {
        assert_eq!(w.get(&group_path, who).await.0, StatusCode::NOT_FOUND);
        assert_eq!(
            w.get(&format!("{group_path}/members"), who).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(transcript(who).await.status_code(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_coach_links_trainingpeaks_athletes_and_members_answer() {
    let roster_reads: Calls = Arc::new(Mutex::new(0));
    let dropped: Dropped = Arc::new(Mutex::new(Vec::new()));
    env::set_var(
        ENV_REMOTE_URL,
        spawn_scraper(roster_reads.clone(), dropped.clone()).await,
    );
    env::remove_var(ENV_AUDIENCE);
    let w = world(roster_reads, dropped).await;

    the_coach_sees_the_group(&w).await;
    only_the_coach_reads_the_roster_with_the_notice_accepted(&w).await;
    an_unread_role_never_serves_a_cached_roster(&w).await;
    the_roster_suggests_members_by_name(&w).await;
    the_roster_read_grants_the_coach_and_is_cached(&w).await;
    a_new_coach_login_drops_the_cached_roster(&w).await;
    every_proposal_rule_refuses_with_its_reason(&w).await;
    let link = the_coach_proposes(&w).await;
    the_member_is_asked(&w).await;
    each_side_lists_what_it_may_see(&w, &link).await;
    only_the_named_member_confirms_without_a_login_of_their_own(&w, &link).await;
    the_confirm_registers_the_link_and_tells_the_coach(&w, &link).await;
    an_own_login_supersedes_the_link(&w, &link).await;
    a_member_declines_and_the_coach_is_told(&w).await;
    the_coach_withdraws_and_revokes(&w).await;
    a_roster_drop_ends_the_link_and_tells_both_sides(&w).await;
    a_live_roster_read_ends_a_proposal_off_the_roster(&w).await;
    an_inactive_group_admits_nobody(&w).await;

    env::remove_var(ENV_REMOTE_URL);
}
