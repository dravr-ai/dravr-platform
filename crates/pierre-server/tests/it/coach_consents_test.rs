// ABOUTME: A member grants and revokes what their group's coach may do beyond reading their workouts, on the coach relationship
// ABOUTME: Pins the grants a confirm names, the member-only change, the API-key refusal, the offer, and the wellness purge a revoke owes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Intervals.icu takes a coach's writes to an athlete's calendar and serves
//! a coach's key the athlete's wellness, so a member coached there is offered
//! both of their own consents: `calendar_write` and `wellness_read`. They live
//! on the member's coach relationship (their membership row), never on a
//! link: each is off until the member grants it, when they confirm a link or
//! at any time under `/members/me/coach-consents`; only the member changes
//! it; a grant is taken only from Dravr's own app, a revoke from anywhere;
//! `wellness_read` is offered only while a confirmed link reads wellness; and
//! revoking it deletes the wellness the member's links synced, leaving the
//! links and their workouts. A link's response mirrors the relationship.
//!
//! The API is a loopback stand-in (a test double, per the repo's mock rule)
//! serving the coach's roster and each athlete's activities to the coach's key.

use crate::common;
use crate::helpers;
use secrecy::SecretString;

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::middleware::from_fn;
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use common::{
    create_test_server_resources_with_config, create_test_user_with_plan, generate_test_token,
    test_server_config_with_api_base,
};
use helpers::axum_test::AxumTestRequest;
use pierre_auth::api_keys::{ApiKey, ApiKeyManager, ApiKeyTier};
use pierre_core::constants::oauth::{INTERVALS_ICU, WHOOP};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    ActivityBuilder, AgentCategory, ConnectionType, CreateAgentRequest, DataSource,
    DelegatedConnection, DelegationStatus, DeviceType, SportType, StoredHealthMetrics,
    StoredRecoveryMetrics, StoredSleepSession, TenantId, UserOAuthToken, API_KEY_TOKEN_TYPE,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::request_transport::declare_request_transport;
use pierre_routes_groups::DelegatedConnectionRoutes;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

/// The coach's own Intervals.icu athlete id and live API key.
const COACH_ATHLETE: &str = "i100";
const COACH_KEY: &str = "coach-key";
/// The three members' athlete ids.
const M1_ATHLETE: &str = "i201";
const M2_ATHLETE: &str = "i202";
const M3_ATHLETE: &str = "i203";

const CONSENTS_MEMBER_ONLY: &str = "Only a member decides what their coach may do";

/// Whether a request carries the coach's live key.
fn coach_key(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|pair| String::from_utf8(pair).ok())
        .is_some_and(|pair| pair == format!("API_KEY:{COACH_KEY}"))
}

async fn athletes_route(headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !coach_key(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    // Each athlete carries the email of the member they are; the coach's own
    // entry binds the key to the coach.
    (
        StatusCode::OK,
        Json(json!([
            { "id": COACH_ATHLETE, "name": "Casey Coach", "email": "coach@consents.test",
              "icu_permission": "WRITE" },
            { "id": M1_ATHLETE, "name": "Alex Athlete", "email": "m1@consents.test",
              "icu_permission": "WRITE", "private_wellness_keys": [] },
            { "id": M2_ATHLETE, "name": "Sam Swimmer", "email": "m2@consents.test",
              "icu_permission": "WRITE", "private_wellness_keys": [] },
            { "id": M3_ATHLETE, "name": "Robin Rower", "email": "m3@consents.test",
              "icu_permission": "WRITE", "private_wellness_keys": [] },
        ])),
    )
}

/// Every athlete's activities, read with the coach's key: none, so the
/// warm-up read after a confirm caches nothing.
async fn activities_route(headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if !coach_key(&headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    (StatusCode::OK, Json(json!([])))
}

async fn spawn_api() -> String {
    let app = Router::new()
        .route("/api/v1/athletes", get(athletes_route))
        .route(
            "/api/v1/athlete/{athlete}/activities",
            get(activities_route),
        );
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
    router: Router,
    group: Uuid,
    /// The tenant the group lives in, its owner's.
    group_tenant: TenantId,
    coach: Person,
    m1: Person,
    m2: Person,
    m3: Person,
    /// A member with no link at all.
    m4: Person,
}

async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    // A verified email: what binds the coach's key, and a roster athlete, to
    // this person.
    repos.email_verification.mark_verified(id).await.unwrap();
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

/// The world, its server reading Intervals.icu at `api_base`.
async fn world(api_base: &str) -> World {
    let res = create_test_server_resources_with_config(test_server_config_with_api_base(
        INTERVALS_ICU,
        api_base,
    ))
    .await
    .unwrap();
    let repos = Arc::clone(&res.common.repos);
    let owner = person(&res, "owner@consents.test", "Olive Owner").await;
    let coach = person(&res, "coach@consents.test", "Casey Coach").await;
    let m1 = person(&res, "m1@consents.test", "Alex Athlete").await;
    let m2 = person(&res, "m2@consents.test", "Sam Swimmer").await;
    let m3 = person(&res, "m3@consents.test", "Robin Rower").await;
    let m4 = person(&res, "m4@consents.test", "Kim Kayaker").await;

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
    for member in [&m1, &m2, &m3, &m4] {
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
                coach_consents: CoachConsents::default(),
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
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: coach.id,
            tenant_id: coach.tenant.to_string(),
            provider: INTERVALS_ICU.to_owned(),
            access_token: SecretString::from(COACH_KEY.to_owned()),
            refresh_token: None,
            token_type: API_KEY_TOKEN_TYPE.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: Some(COACH_ATHLETE.to_owned()),
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
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();

    // Every request is served over the transport its credential names, as
    // the production app declares it.
    let router = DelegatedConnectionRoutes::routes(Arc::clone(&res))
        .layer(from_fn(declare_request_transport));
    World {
        res,
        router,
        group,
        group_tenant: owner.tenant,
        coach,
        m1,
        m2,
        m3,
        m4,
    }
}

/// A stored API key for `user_id`, and the full key to present.
async fn api_key(res: &Arc<ServerContext>, user_id: Uuid) -> String {
    let data = ApiKeyManager::new().generate_api_key(false);
    let key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "consent key".to_owned(),
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
    res.common.repos.api_keys.create(&key).await.unwrap();
    data.full_key
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// Assert the consents a link response carries, and those its platform
/// offers: `[calendar_write, calendar_write_supported, wellness_read,
/// wellness_read_supported]`; and that no source of the member's own skips
/// its wellness.
fn assert_consents(link: &Value, expected: [bool; 4]) {
    let got = [
        "calendar_write",
        "calendar_write_supported",
        "wellness_read",
        "wellness_read_supported",
    ]
    .map(|field| {
        link[field]
            .as_bool()
            .unwrap_or_else(|| panic!("{field}: {link}"))
    });
    assert_eq!(got, expected, "{link}");
    assert_eq!(
        link.get("wellness_read_skipped"),
        Some(&Value::Null),
        "{link}"
    );
}

/// The link `id` in the group's link list, as `who` reads it.
async fn listed(w: &World, who: &Person, id: Uuid) -> Value {
    let resp = AxumTestRequest::get(&w.links_path())
        .header("authorization", &who.auth)
        .send(w.router.clone())
        .await;
    let listed = parse(&resp.text());
    listed["connections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id.to_string())
        .cloned()
        .unwrap_or_else(|| panic!("{id} is listed: {listed}"))
}

impl World {
    fn links_path(&self) -> String {
        format!("/api/groups/{}/delegated-connections", self.group)
    }

    fn consents_path(&self) -> String {
        format!("/api/groups/{}/members/me/coach-consents", self.group)
    }

    async fn send(
        &self,
        request: AxumTestRequest,
        authorization: &str,
        body: &Value,
    ) -> (StatusCode, Value) {
        let resp = request
            .header("authorization", authorization)
            .json(body)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    async fn put_consents(&self, authorization: &str, body: &Value) -> (StatusCode, Value) {
        self.send(
            AxumTestRequest::put(&self.consents_path()),
            authorization,
            body,
        )
        .await
    }

    async fn get_consents(&self, authorization: &str) -> (StatusCode, Value) {
        let resp = AxumTestRequest::get(&self.consents_path())
            .header("authorization", authorization)
            .send(self.router.clone())
            .await;
        (resp.status_code(), parse(&resp.text()))
    }

    /// `member`'s coach consents as stored on their membership.
    async fn consents(&self, member: &Person) -> CoachConsents {
        self.res
            .common
            .repos
            .groups
            .get_member(&self.group.to_string(), member.id)
            .await
            .unwrap()
            .expect("a live member")
            .coach_consents
    }

    /// The coach proposes `athlete` as `member`, and the member confirms with
    /// `body`, or with no body at all when `None`, as today's clients send
    /// it. Returns the link id and the confirm's response.
    async fn link(&self, athlete: &str, member: &Person, body: Option<&Value>) -> (Uuid, Value) {
        let (status, proposed) = self
            .send(
                AxumTestRequest::post(&self.links_path()),
                &self.coach.auth,
                &json!({
                    "provider": "intervals_icu",
                    "provider_athlete_id": athlete,
                    "member_user_id": member.id,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{proposed}");
        // A proposal holds no consent, and shows what the platform offers.
        assert_consents(&proposed, [false, true, false, true]);
        let id = Uuid::parse_str(proposed["id"].as_str().unwrap()).unwrap();
        let confirm = AxumTestRequest::post(&format!("{}/{id}/confirm", self.links_path()))
            .header("authorization", &member.auth);
        let confirm = match body {
            Some(body) => confirm.json(body),
            None => confirm,
        };
        let resp = confirm.send(self.router.clone()).await;
        let (status, confirmed) = (resp.status_code(), parse(&resp.text()));
        assert_eq!(status, StatusCode::OK, "{confirmed}");
        assert_eq!(confirmed["status"], "confirmed");
        (id, confirmed)
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
            .expect("the link stays for audit")
    }

    /// Seed the wellness and the one workout a link synced for `member`, as
    /// the sync files them: under the member's tenant, as Intervals.icu's.
    async fn seed_synced(&self, member: &Person) {
        let repos = &self.res.common.repos;
        let source = repos
            .data_sources
            .upsert_data_source(
                &member.tenant,
                &DataSource {
                    id: String::new(),
                    user_id: member.id.to_string(),
                    provider: INTERVALS_ICU.to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Band,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        repos
            .sleep
            .upsert_sleep_session(
                &member.tenant,
                &StoredSleepSession {
                    id: format!("icu-sleep-{}", Uuid::new_v4()),
                    user_id: member.id.to_string(),
                    data_source_id: source.clone(),
                    is_nap: false,
                    start_datetime: night(),
                    end_datetime: night() + Duration::hours(8),
                    total_sleep_seconds: Some(25_200),
                    deep_sleep_seconds: None,
                    light_sleep_seconds: None,
                    rem_sleep_seconds: None,
                    awake_seconds: None,
                    sleep_efficiency: None,
                    avg_heart_rate: None,
                    min_heart_rate: None,
                    avg_hrv: Some(68.0),
                    sleep_score: None,
                    stages: Vec::new(),
                    source_name: INTERVALS_ICU.to_owned(),
                },
            )
            .await
            .unwrap();
        repos
            .recovery
            .upsert_recovery_metrics(
                &member.tenant,
                &StoredRecoveryMetrics {
                    id: format!("icu-day-{}", Uuid::new_v4()),
                    user_id: member.id.to_string(),
                    data_source_id: source.clone(),
                    date: morning(),
                    recovery_score: None,
                    readiness_score: None,
                    hrv_ms: Some(68.0),
                    hrv_rmssd: Some(68.0),
                    resting_heart_rate: Some(48),
                    stress_score: None,
                    body_battery: None,
                    spo2: None,
                    respiratory_rate: None,
                    skin_temp_deviation: None,
                    daily_strain: None,
                    athlete_note: None,
                    source_name: INTERVALS_ICU.to_owned(),
                    recorded_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        repos
            .health_snapshots
            .upsert_health_snapshot(
                &member.tenant,
                &StoredHealthMetrics {
                    id: String::new(),
                    user_id: member.id.to_string(),
                    data_source_id: source,
                    date: morning(),
                    weight_kg: Some(68.2),
                    body_fat_pct: None,
                    muscle_mass_kg: None,
                    bmi: None,
                    bone_mass_kg: None,
                    water_pct: None,
                    systolic_bp: None,
                    diastolic_bp: None,
                    blood_glucose: None,
                    vo2_max: None,
                    source_name: INTERVALS_ICU.to_owned(),
                    recorded_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let ride = ActivityBuilder::new(
            format!("icu-ride-{}", member.id),
            "Endurance ride".to_owned(),
            SportType::Ride,
            night() + Duration::hours(14),
            5_400,
            INTERVALS_ICU.to_owned(),
        )
        .build();
        repos
            .activity_cache
            .upsert_activities(member.id, &member.tenant, INTERVALS_ICU, &[ride])
            .await
            .unwrap();
    }

    /// How many sleep, recovery and health rows, and cached workouts, the
    /// link holds for `member`.
    async fn synced(&self, member: &Person) -> [usize; 4] {
        let repos = &self.res.common.repos;
        let (from, to) = (night() - Duration::days(1), night() + Duration::days(2));
        [
            repos
                .sleep
                .get_sleep_sessions(member.id, &member.tenant, from, to)
                .await
                .unwrap()
                .len(),
            repos
                .recovery
                .get_recovery_metrics(member.id, &member.tenant, from, to)
                .await
                .unwrap()
                .len(),
            repos
                .health_snapshots
                .get_health_snapshots(member.id, &member.tenant, from, to)
                .await
                .unwrap()
                .len(),
            repos
                .activity_cache
                .get_cached_activities(member.id, &member.tenant, Some(INTERVALS_ICU), from, to, 10)
                .await
                .unwrap()
                .len(),
        ]
    }
}

fn night() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 20, 22, 0, 0).unwrap()
}

fn morning() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 21).unwrap()
}

/// A confirm grants exactly the consents it names, each stamped on the
/// member's relationship, and one with no body grants nothing, though the
/// platform offers both.
async fn a_confirm_grants_what_it_names(w: &World) -> (Uuid, Uuid, Uuid) {
    let (m1_link, confirmed) = w
        .link(M1_ATHLETE, &w.m1, Some(&json!({ "calendar_write": true })))
        .await;
    assert_consents(&confirmed, [true, true, false, true]);
    let stored = w.consents(&w.m1).await;
    assert!(stored.calendar_write && stored.calendar_write_at.is_some());
    assert_eq!(stored.wellness_read_at, None);

    let (m2_link, confirmed) = w
        .link(
            M2_ATHLETE,
            &w.m2,
            Some(&json!({ "calendar_write": true, "wellness_read": true })),
        )
        .await;
    assert_consents(&confirmed, [true, true, true, true]);
    assert!(w.consents(&w.m2).await.wellness_read_at.is_some());

    let (m3_link, confirmed) = w.link(M3_ATHLETE, &w.m3, None).await;
    assert_consents(&confirmed, [false, true, false, true]);
    assert_eq!(w.stored(m3_link).await.status, DelegationStatus::Confirmed);
    assert_eq!(
        w.consents(&w.m3).await,
        CoachConsents::default(),
        "a confirm with no body grants nothing"
    );
    (m1_link, m2_link, m3_link)
}

/// A member with no link is offered calendar writes (the group has a coach,
/// and Intervals.icu takes a coach's writes) but no wellness sync, since no
/// link of theirs reads it: a wellness grant is ignored, a calendar grant
/// stands. The coach reads no member's consents through this path, and a
/// coach's or an outsider's change is refused.
async fn the_relationship_offers_what_applies(w: &World) {
    let (status, body) = w.get_consents(&w.m4.auth).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [false, true, false, false]);

    let (status, body) = w
        .put_consents(
            &w.m4.auth,
            &json!({ "calendar_write": true, "wellness_read": true }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [true, true, false, false]);
    assert!(body["calendar_write_at"].is_string(), "{body}");
    let stored = w.consents(&w.m4).await;
    assert!(stored.calendar_write && !stored.wellness_read);
    assert_eq!(
        stored.wellness_read_at, None,
        "an ignored grant stamps nothing"
    );

    let (status, body) = w.get_consents(&w.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the coach is no member: {body}"
    );
    let (status, body) = w
        .put_consents(&w.coach.auth, &json!({ "calendar_write": true }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["message"], CONSENTS_MEMBER_ONLY);
}

/// A WHOOP connection of the member's own wins over what the link would
/// sync: the link and the relationship tell the member so whether or not
/// they granted it, and the coach only once they did, since which sources a
/// member connects is theirs until they share their wellness. The consent
/// stays theirs to change. Another member's link says nothing of it.
async fn an_own_wellness_source_is_named_on_the_link(w: &World, m3_link: Uuid, m1_link: Uuid) {
    w.res
        .common
        .repos
        .provider_connections
        .register_connection(w.m3.id, w.m3.tenant, WHOOP, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let row = listed(w, &w.m3, m3_link).await;
    assert_eq!(row["wellness_read"], false, "{row}");
    assert_eq!(row["wellness_read_skipped"], "own_wellness_source", "{row}");
    let row = listed(w, &w.coach, m3_link).await;
    assert_eq!(
        row.get("wellness_read_skipped"),
        Some(&Value::Null),
        "the coach does not learn of an unshared source: {row}"
    );

    let (status, body) = w
        .put_consents(&w.m3.auth, &json!({ "wellness_read": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["wellness_read"], true, "{body}");
    assert_eq!(
        body["wellness_read_skipped"], "own_wellness_source",
        "{body}"
    );

    let row = listed(w, &w.coach, m3_link).await;
    assert_eq!(row["wellness_read"], true, "{row}");
    assert_eq!(row["wellness_read_skipped"], "own_wellness_source", "{row}");
    let other = listed(w, &w.coach, m1_link).await;
    assert_eq!(other.get("wellness_read_skipped"), Some(&Value::Null));
}

/// The member grants and revokes on their relationship, one consent at a
/// time; the coach reads the state through the link and cannot change it,
/// and another member's change is their own, leaving this member's alone.
async fn only_the_member_changes_what_their_coach_may_do(w: &World, link: Uuid) {
    let (status, body) = w
        .put_consents(&w.m1.auth, &json!({ "wellness_read": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [true, true, true, true]);
    let (status, body) = w
        .put_consents(&w.m1.auth, &json!({ "calendar_write": false }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [false, true, true, true]);
    let (status, body) = w.get_consents(&w.m1.auth).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [false, true, true, true]);

    let (status, body) = w
        .put_consents(&w.coach.auth, &json!({ "calendar_write": true }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["message"], CONSENTS_MEMBER_ONLY);
    let (status, body) = w
        .put_consents(&w.m2.auth, &json!({ "calendar_write": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "m2 sets their own: {body}");
    let stored = w.consents(&w.m1).await;
    assert!(!stored.calendar_write && stored.wellness_read);

    // The coach sees what the member granted, on the link.
    assert_consents(&listed(w, &w.coach, link).await, [false, true, true, true]);

    // The per-link consent route is gone: a link carries no consent.
    let (status, _) = w
        .send(
            AxumTestRequest::put(&format!("{}/{link}/consents", w.links_path())),
            &w.m1.auth,
            &json!({ "calendar_write": true }),
        )
        .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
        "{status}"
    );
    assert!(!w.consents(&w.m1).await.calendar_write);
}

/// Over the member's API key a grant is refused and writes nothing; a revoke
/// goes through.
async fn an_api_key_revokes_but_never_grants(w: &World) {
    let key = api_key(&w.res, w.m1.id).await;
    let (status, body) = w
        .put_consents(&key, &json!({ "calendar_write": true }))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        body["details"]["reason"], "consent_grant_needs_app",
        "{body}"
    );
    assert!(
        !w.consents(&w.m1).await.calendar_write,
        "nothing was granted"
    );

    let (status, body) = w
        .put_consents(&key, &json!({ "calendar_write": false }))
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a revoke works from anywhere: {body}"
    );
}

/// Revoking the wellness consent deletes the wellness the member's links
/// synced and records it; the link, its workouts, the calendar consent and
/// the other member's wellness stay. A calendar revoke deletes nothing.
async fn revoking_wellness_deletes_what_the_links_synced(w: &World, m1_link: Uuid, m2_link: Uuid) {
    w.seed_synced(&w.m1).await;
    w.seed_synced(&w.m2).await;
    assert_eq!(w.synced(&w.m2).await, [1, 1, 1, 1]);

    let (status, body) = w
        .put_consents(&w.m2.auth, &json!({ "calendar_write": false }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        w.synced(&w.m2).await,
        [1, 1, 1, 1],
        "a calendar revoke deletes no wellness"
    );

    let key = api_key(&w.res, w.m2.id).await;
    let (status, body) = w
        .put_consents(&key, &json!({ "wellness_read": false }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [false, true, false, true]);
    assert_eq!(
        w.synced(&w.m2).await,
        [0, 0, 0, 1],
        "the wellness goes, the workouts stay"
    );
    assert_eq!(w.stored(m2_link).await.status, DelegationStatus::Confirmed);
    assert_eq!(
        w.synced(&w.m1).await,
        [1, 1, 1, 1],
        "another member's wellness is untouched"
    );
    assert_eq!(w.stored(m1_link).await.status, DelegationStatus::Confirmed);
    assert!(w.consents(&w.m1).await.wellness_read);

    the_withdrawal_is_recorded(w, &w.m2).await;
}

/// One purge is recorded for `member`'s withdrawn wellness consent, in their
/// tenant, naming every row it removed.
async fn the_withdrawal_is_recorded(w: &World, member: &Person) {
    let recorded = w
        .res
        .common
        .repos
        .provider_data
        .list_provider_data_purges(INTERVALS_ICU, 10)
        .await
        .unwrap();
    let withdrawn: Vec<_> = recorded
        .iter()
        .filter(|r| r.user_id.as_deref() == Some(member.id.to_string().as_str()))
        .collect();
    assert_eq!(withdrawn.len(), 1, "{recorded:?}");
    assert_eq!(withdrawn[0].reason, "wellness_consent_withdrawn");
    assert_eq!(withdrawn[0].tenant_id, Some(member.tenant.to_string()));
    // The sleep, recovery and health rows, and the data source they named.
    assert_eq!(withdrawn[0].rows_removed, 4);
}

/// A group with no coach offers no consent: a member's grant is ignored and
/// stamps nothing, so no consent waits for a coach the member never saw. What
/// they granted before stays theirs to revoke (`m3` granted `wellness_read`).
async fn a_group_with_no_coach_offers_nothing(w: &World) {
    assert!(w
        .res
        .common
        .repos
        .groups
        .set_group_coach_user(&w.group.to_string(), None, w.group_tenant)
        .await
        .unwrap());
    let before = w.consents(&w.m3).await;
    assert!(!before.calendar_write, "{before:?}");

    let (status, body) = w.get_consents(&w.m3.auth).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [false, false, true, false]);

    let (status, body) = w
        .put_consents(&w.m3.auth, &json!({ "calendar_write": true }))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_consents(&body, [false, false, true, false]);
    assert_eq!(
        w.consents(&w.m3).await,
        before,
        "an ignored grant stores nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_grants_and_revokes_what_their_coach_may_do() {
    let w = world(&spawn_api().await).await;

    let (m1_link, m2_link, m3_link) = a_confirm_grants_what_it_names(&w).await;
    the_relationship_offers_what_applies(&w).await;
    only_the_member_changes_what_their_coach_may_do(&w, m1_link).await;
    an_api_key_revokes_but_never_grants(&w).await;
    revoking_wellness_deletes_what_the_links_synced(&w, m1_link, m2_link).await;
    an_own_wellness_source_is_named_on_the_link(&w, m3_link, m1_link).await;
    a_group_with_no_coach_offers_nothing(&w).await;
}
