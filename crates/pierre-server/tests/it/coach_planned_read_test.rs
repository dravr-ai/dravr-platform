// ABOUTME: A human coach reads a linked athlete's Intervals.icu planned calendar with get_planned_workouts(athlete=…)
// ABOUTME: Pins the coach's key at the athlete's path, the hidden drafts the coach alone sees, each refusal before any read, and the link's reactions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(all(feature = "provider-intervals-icu", feature = "tools-data"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A group's human coach names a coached athlete with `athlete=` on
//! `get_planned_workouts` and reads that athlete's Intervals.icu calendar
//! through the coach's own API key, at the athlete's path: the path a link
//! the athlete confirmed names, or the athlete's own Intervals.icu id when
//! they connect it themselves (carnet#1055). The read needs no consent beyond
//! the athlete's sharing with the coach (carnet#1066): every athlete here
//! keeps `calendar_write` off.
//!
//! The coach is the reader, so the sessions the coach hides from the athlete
//! come back titled "Hidden from athlete: …", while the athlete's own read of
//! the same calendar, through the same link, leaves them out.
//!
//! Intervals.icu is a loopback stand-in (a test double, per the repo's mock
//! rule: the real provider's wire shape is pinned by
//! `intervals_icu_delegated_read_test` in pierre-providers) that records the
//! path and key of every request. Every refusal is asserted with no call to
//! any athlete's calendar.

use crate::common;

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth::providers::INTERVALS_ICU;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{
    ConnectionStatus, ConnectionType, DelegatedConnection, RosterAthlete, TenantId, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use pierre_core::transport::Transport;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::ai_scope::serve_over;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::planned_workouts::GetPlannedWorkoutsTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uuid::Uuid;

const COACH_EMAIL: &str = "coach@planned-read.test";
/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// The coach's API key, the one the stand-in honours.
const COACH_KEY: &str = "coach-key";

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Hit {
    path: String,
    key: String,
}

/// The stand-in's record and what it answers.
#[derive(Clone, Default)]
struct Api {
    hits: Arc<Mutex<Vec<Hit>>>,
    /// Athletes the coach's key may no longer read: their calendar answers
    /// 403, as Intervals.icu answers for an athlete off the coach's roster.
    off_roster: Arc<Mutex<HashSet<String>>>,
    /// Whether the coach's key has stopped working, as a regenerated key
    /// does: every request is then refused as unauthenticated.
    dead: Arc<AtomicBool>,
}

impl Api {
    /// Record a request to `path`, returning whether its key is the
    /// coach's, live.
    fn record(&self, path: String, headers: &HeaderMap) -> bool {
        let key = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Basic "))
            .and_then(|encoded| STANDARD.decode(encoded).ok())
            .and_then(|pair| String::from_utf8(pair).ok())
            .and_then(|pair| pair.strip_prefix("API_KEY:").map(str::to_owned))
            .unwrap_or_default();
        let coach = key == COACH_KEY && !self.dead.load(Ordering::SeqCst);
        self.hits.lock().unwrap().push(Hit { path, key });
        coach
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.hits.lock().unwrap().clear();
    }

    /// Every call to an athlete's calendar or activities.
    fn calendar_calls(&self) -> Vec<Hit> {
        self.hits()
            .into_iter()
            .filter(|h| h.path.starts_with("/api/v1/athlete/"))
            .collect()
    }
}

type Answer = (StatusCode, Json<Value>);

/// The athlete's calendar: a session they did, a race, one of Dravr's own
/// pushes, and a draft the coach keeps hidden from them, which they did too.
fn calendar() -> Value {
    json!([
        { "id": 42, "category": "WORKOUT", "name": "Threshold 3x10", "type": "Ride",
          "description": "Hold the line.", "start_date_local": "2026-09-29T00:00:00",
          "moving_time": 3600 },
        { "id": 43, "category": "RACE_A", "name": "Fall Classic", "type": "Run",
          "start_date_local": "2026-10-04" },
        { "id": 45, "category": "WORKOUT", "name": "Dravr session",
          "start_date_local": "2026-09-30T00:00:00",
          "external_id": "dravr:plan:4b0f3c1e-8a2d-4f6b-9c3e-2d1a7e5f9b10:2026-09-30:0" },
        { "id": 46, "category": "WORKOUT", "name": "Coach draft", "type": "Ride",
          "hide_from_athlete": true, "start_date_local": "2026-10-01T00:00:00" }
    ])
}

async fn events_route(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> Answer {
    if !api.record(format!("/api/v1/athlete/{athlete}/events"), &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    if api.off_roster.lock().unwrap().contains(&athlete) {
        return (StatusCode::FORBIDDEN, Json(json!({})));
    }
    (StatusCode::OK, Json(calendar()))
}

/// The pairing, null values left out as `fields=` leaves them.
async fn activities_route(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> Answer {
    if !api.record(format!("/api/v1/athlete/{athlete}/activities"), &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({})));
    }
    (
        StatusCode::OK,
        Json(json!([
            { "id": "i101", "paired_event_id": 42 },
            { "id": "i102", "paired_event_id": 46 },
            { "id": "i103" }
        ])),
    )
}

async fn spawn_api(api: Api) -> String {
    let app = Router::new()
        .route("/api/v1/athlete/{athlete}/events", get(events_route))
        .route(
            "/api/v1/athlete/{athlete}/activities",
            get(activities_route),
        )
        .with_state(api);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

#[derive(Debug, Clone, Copy)]
struct Person {
    id: Uuid,
    tenant: TenantId,
}

struct World {
    res: Arc<ServerContext>,
    api: Api,
    owner: Person,
    coach: Person,
    /// The group the coach coaches.
    group: CoachingGroup,
    /// The agent the coach's direct chat answers as.
    coach_agent: String,
}

/// A verified user whose email is `email` and display name `name`, in a
/// tenant of their own.
async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, _, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    Person { id, tenant }
}

/// `who`'s own Intervals.icu API key, for account `athlete`, with the
/// connection a linked key registers.
async fn connect_intervals(repos: &RepositoryRegistry, who: Person, key: &str, athlete: &str) {
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: who.id,
            tenant_id: who.tenant.to_string(),
            provider: INTERVALS_ICU.to_owned(),
            access_token: key.to_owned(),
            refresh_token: None,
            token_type: API_KEY_TOKEN_TYPE.to_owned(),
            expires_at: None,
            scope: None,
            provider_user_id: Some(athlete.to_owned()),
            oauth_app_client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(
            who.id,
            who.tenant,
            INTERVALS_ICU,
            &ConnectionType::Manual,
            None,
        )
        .await
        .unwrap();
}

/// An active group `owner` owns and `coach` coaches.
async fn coached_group(
    res: &Arc<ServerContext>,
    owner: Person,
    coach: Person,
    name: &str,
) -> CoachingGroup {
    let repos = &res.common.repos;
    let agent_id = create_group_agent(repos, owner.id, owner.tenant)
        .await
        .unwrap();
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
                agent_id,
                owner_id: owner.id,
                coach_user_id: None,
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    assert!(repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    group
}

impl World {
    /// A server whose Intervals.icu calls go to the stand-in; a group the
    /// coach coaches; the coach's own Intervals.icu key.
    async fn new() -> Self {
        let api = Api::default();
        let base = spawn_api(api.clone()).await;
        let resources = create_test_server_resources().await.unwrap();
        let mut registry = ProviderRegistry::new();
        let mut config = default_config();
        config.api_base_url = base;
        registry.set_default_config(INTERVALS_ICU, config);
        let mut context = (*resources).clone();
        context.fitness.provider_registry = Arc::new(registry);
        let res = Arc::new(context);

        let owner = person(&res, "owner@planned-read.test", "Olive Owner").await;
        let coach = person(&res, COACH_EMAIL, "Casey Coach").await;
        let repos = &res.common.repos;
        connect_intervals(repos, coach, COACH_KEY, COACH_ATHLETE).await;
        let coach_agent = create_group_agent(repos, coach.id, coach.tenant)
            .await
            .unwrap();
        let group = coached_group(&res, owner, coach, "Tempo Squad").await;
        Self {
            res,
            api,
            owner,
            coach,
            group,
            coach_agent,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// `who` joins `group`, sharing with its coach or not.
    async fn join(&self, group: &CoachingGroup, who: Person, shares: bool) {
        let now = Utc::now();
        self.repos()
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: who.id,
                tenant_id: who.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: shares,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// A member of the coach's group named `name`, sharing with the coach
    /// and linked as roster athlete `athlete` through the coach's key, with
    /// calendar writes left off.
    async fn athlete(&self, name: &str, athlete: &str) -> Person {
        let member = person(&self.res, &format!("{athlete}@planned-read.test"), name).await;
        self.join(&self.group, member, true).await;
        self.link(self.coach, &self.group, member, athlete).await;
        member
    }

    /// A confirmed Intervals.icu link of `coach`'s in `group` for roster
    /// athlete `athlete` as `member`, with no per-link consent, and the
    /// connection it registers.
    async fn link(
        &self,
        coach: Person,
        group: &CoachingGroup,
        member: Person,
        athlete: &str,
    ) -> DelegatedConnection {
        let repos = self.repos();
        let proposed = repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                INTERVALS_ICU.to_owned(),
                group.id,
                coach.id,
                coach.tenant,
                member.id,
                RosterAthlete::new(athlete, None, Some(format!("{athlete}@planned-read.test"))),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet");
        let link = repos
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
                INTERVALS_ICU,
                &ConnectionType::Delegated,
                Some(&json!({ "delegated_connection_id": link.id }).to_string()),
            )
            .await
            .unwrap();
        link
    }

    /// The coach's direct chat: a conversation under their own tenant,
    /// bound to their agent and to no group.
    async fn coach_dm(&self) -> String {
        self.repos()
            .chat
            .create_conversation(
                &self.coach.id.to_string(),
                self.coach.tenant,
                "coach dm",
                "gemini-2.0-flash",
                Some(&self.coach_agent),
                None,
            )
            .await
            .unwrap()
            .id
    }

    /// `get_planned_workouts` as `who` in their own tenant, from Dravr's web
    /// app: its payload, and whether it refused.
    async fn call(&self, who: Person, args: Value) -> (Value, bool) {
        let runtime: Arc<dyn ToolRuntime> = Arc::clone(&self.res) as Arc<dyn ToolRuntime>;
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(who.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response = serve_over(
            Transport::WebApp,
            GetPlannedWorkoutsTool.execute(&runtime, &ctx, args),
        )
        .await;
        (
            response
                .structured_content
                .expect("tool result carries structured content"),
            response.is_error,
        )
    }

    /// The coach's read of `athlete`'s calendar over the stand-in's week,
    /// from their direct chat.
    async fn coach_reads(&self, athlete: &str) -> (Value, bool) {
        let conversation = self.coach_dm().await;
        CONVERSATION_ID
            .scope(Some(conversation), self.call(self.coach, week_of(athlete)))
            .await
    }

    /// Whether `member`'s link is still live.
    async fn linked(&self, member: Person) -> bool {
        self.repos()
            .delegated_connections
            .find_active_for_member(member.id, member.tenant, INTERVALS_ICU)
            .await
            .unwrap()
            .is_some()
    }

    /// The status of `who`'s Intervals.icu connection.
    async fn connection_status(&self, who: Person) -> ConnectionStatus {
        self.repos()
            .provider_connections
            .get_for_user(who.id, Some(who.tenant))
            .await
            .unwrap()
            .into_iter()
            .find(|c| c.provider == INTERVALS_ICU)
            .expect("an Intervals.icu connection")
            .status
    }
}

/// The arguments of a read of the stand-in's week, for `athlete`.
fn week_of(athlete: &str) -> Value {
    json!({ "athlete": athlete, "start_date": "2026-09-28", "end_date": "2026-10-04" })
}

/// Each workout's (date, title, completing activity), in the order the model
/// reads them.
fn read(payload: &Value) -> Vec<(String, String, Option<String>)> {
    payload["planned_workouts"]
        .as_array()
        .unwrap_or_else(|| panic!("expected planned workouts, got {payload}"))
        .iter()
        .map(|w| {
            (
                w["date"].as_str().unwrap().to_owned(),
                w["title"].as_str().unwrap().to_owned(),
                w["completed_activity_id"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

fn workout(date: &str, title: &str, done: Option<&str>) -> (String, String, Option<String>) {
    (date.to_owned(), title.to_owned(), done.map(str::to_owned))
}

fn reason(payload: &Value) -> &str {
    payload["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a named refusal, got {payload}"))
}

fn error_text(payload: &Value) -> &str {
    payload["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal, got {payload}"))
}

#[tokio::test]
async fn a_coach_reads_a_linked_athletes_calendar_hidden_drafts_and_all_with_the_coach_key() {
    let w = World::new().await;
    let alex = w.athlete("Alex Athlete", "i201").await;

    let (payload, refused) = w.coach_reads("Alex").await;
    assert!(!refused, "{payload}");
    assert_eq!(payload["athlete"], "Alex Athlete", "{payload}");
    assert_eq!(payload["providers"], json!(["intervals_icu"]), "{payload}");
    assert_eq!(
        read(&payload),
        [
            workout("2026-09-29", "Threshold 3x10", Some("i101")),
            workout(
                "2026-10-01",
                "Hidden from athlete: Coach draft",
                Some("i102")
            ),
            workout("2026-10-04", "A race: Fall Classic", None),
        ],
        "the coach reads the draft they hide, labelled, and never Dravr's own push"
    );
    let calls = w.api.calendar_calls();
    assert!(!calls.is_empty());
    for call in &calls {
        assert_eq!(call.key, COACH_KEY, "{calls:?}");
        assert!(
            call.path.starts_with("/api/v1/athlete/i201/"),
            "only the athlete's own path is read: {calls:?}"
        );
    }

    // Naming the platform reads the same calendar; naming one that keeps no
    // planned calendar is refused with the ones that do.
    let conversation = w.coach_dm().await;
    let mut named = week_of("Alex");
    named["provider"] = json!("intervals_icu");
    let (narrowed, refused) = CONVERSATION_ID
        .scope(Some(conversation.clone()), w.call(w.coach, named.clone()))
        .await;
    assert!(!refused, "{narrowed}");
    assert_eq!(read(&narrowed), read(&payload));
    named["provider"] = json!("strava");
    let (refusal, refused) = CONVERSATION_ID
        .scope(Some(conversation), w.call(w.coach, named))
        .await;
    assert!(refused, "{refusal}");
    assert!(
        refusal.to_string().contains("intervals_icu"),
        "names the providers that keep a planned calendar: {refusal}"
    );

    // The athlete's own read of the same calendar, through the same link,
    // leaves the coach's draft out.
    w.api.clear();
    let (own, refused) = w
        .call(
            alex,
            json!({ "start_date": "2026-09-28", "end_date": "2026-10-04" }),
        )
        .await;
    assert!(!refused, "{own}");
    assert!(own.get("athlete").is_none(), "{own}");
    assert_eq!(
        read(&own),
        [
            workout("2026-09-29", "Threshold 3x10", Some("i101")),
            workout("2026-10-04", "A race: Fall Classic", None),
        ]
    );
    assert!(w
        .api
        .calendar_calls()
        .iter()
        .all(|call| call.key == COACH_KEY && call.path.starts_with("/api/v1/athlete/i201/")));
}

#[tokio::test]
async fn every_refusal_is_named_and_reaches_no_athletes_calendar() {
    let w = World::new().await;
    let alex = w.athlete("Alex Athlete", "i201").await;
    // An athlete who stopped sharing with the coach.
    let sara = person(&w.res, "i231@planned-read.test", "Sara Stopped").await;
    w.join(&w.group, sara, false).await;
    w.link(w.coach, &w.group, sara, "i231").await;
    // A member with no coaching platform at all.
    let nell = person(&w.res, "nell@planned-read.test", "Nell Nolink").await;
    w.join(&w.group, nell, true).await;
    // An athlete linked through another coach, who is also in this coach's
    // group.
    let blake = person(&w.res, "blake@planned-read.test", "Blake Othercoach").await;
    connect_intervals(w.repos(), blake, "blake-key", "i400").await;
    let other_group = coached_group(&w.res, w.owner, blake, "Other Squad").await;
    let bea = person(&w.res, "i216@planned-read.test", "Bea Elsewhere").await;
    w.join(&other_group, bea, true).await;
    w.join(&w.group, bea, true).await;
    w.link(blake, &other_group, bea, "i216").await;

    let (refusal, refused) = w.coach_reads("Sara").await;
    assert!(refused);
    assert!(
        error_text(&refusal).contains("stopped sharing"),
        "{refusal}"
    );

    let (refusal, refused) = w.coach_reads("Nell").await;
    assert!(refused);
    assert_eq!(reason(&refusal), "not_linked", "{refusal}");
    assert!(error_text(&refusal).contains("propose_athlete_link"));

    let (refusal, refused) = w.coach_reads("Bea").await;
    assert!(refused);
    assert_eq!(
        reason(&refusal),
        "linked_through_another_coach",
        "{refusal}"
    );
    assert!(
        error_text(&refusal).contains("only that coach reads their calendar"),
        "{refusal}"
    );

    // A name no athlete the caller coaches answers to — and a member, who
    // coaches no one, reads no one's calendar.
    let (refusal, refused) = w.coach_reads("Zed").await;
    assert!(refused);
    assert!(
        error_text(&refusal).contains("No athlete matching"),
        "{refusal}"
    );
    let (refusal, refused) = w.call(alex, week_of("Bea")).await;
    assert!(refused);
    assert!(
        error_text(&refusal).contains("No athlete matching"),
        "{refusal}"
    );

    // A group's room is read by every member.
    let (refusal, refused) = CONVERSATION_ID
        .scope(
            Some("a-room-the-coachs-tenant-cannot-resolve".to_owned()),
            w.call(w.coach, week_of("Alex")),
        )
        .await;
    assert!(refused);
    assert_eq!(reason(&refusal), "group_room", "{refusal}");

    assert!(
        w.api.calendar_calls().is_empty(),
        "no refusal reached a calendar: {:?}",
        w.api.calendar_calls()
    );
}

#[tokio::test]
async fn an_athlete_off_the_coachs_roster_ends_the_link_on_the_read() {
    let w = World::new().await;
    let gale = w.athlete("Gale Gone", "i240").await;
    w.api.off_roster.lock().unwrap().insert("i240".to_owned());

    let (refusal, refused) = w.coach_reads("Gale").await;
    assert!(refused, "{refusal}");
    assert_eq!(refusal["provider"], "intervals_icu", "{refusal}");
    assert!(
        !w.linked(gale).await,
        "the platform no longer lets the coach read the athlete, so the link ends"
    );

    // The next read finds no link to go through.
    w.api.clear();
    let (refusal, refused) = w.coach_reads("Gale").await;
    assert!(refused, "{refusal}");
    assert!(w.api.calendar_calls().is_empty(), "{:?}", w.api.hits());
}

#[tokio::test]
async fn a_dead_coach_key_flags_the_coach_and_the_next_read_names_the_reconnect() {
    let w = World::new().await;
    let alex = w.athlete("Alex Athlete", "i201").await;
    w.api.dead.store(true, Ordering::SeqCst);

    let (refusal, refused) = w.coach_reads("Alex").await;
    assert!(refused, "{refusal}");
    assert_eq!(
        w.connection_status(w.coach).await,
        ConnectionStatus::NeedsReauth,
        "the coach's own connection is the one to renew"
    );
    assert_eq!(
        w.connection_status(alex).await,
        ConnectionStatus::Active,
        "never the athlete's"
    );
    assert!(w.linked(alex).await, "a dead coach key ends no link");

    // The next read is refused before any call, telling the coach to
    // reconnect.
    w.api.clear();
    let (refusal, refused) = w.coach_reads("Alex").await;
    assert!(refused, "{refusal}");
    assert_eq!(
        reason(&refusal),
        "coach_platform_reconnect_needed",
        "{refusal}"
    );
    assert!(w.api.calendar_calls().is_empty(), "{:?}", w.api.hits());
}

// ════════════════════════════════════════════════════════════════════════
// An athlete who connects Intervals.icu themselves (carnet#1055)
// ════════════════════════════════════════════════════════════════════════

/// A coached member who connects Intervals.icu with a key of their own, as
/// account `athlete`: no link, and no calendar-write consent.
async fn own_connection(w: &World, name: &str, athlete: &str, shares: bool) -> Person {
    let member = person(&w.res, &format!("{athlete}@planned-read.test"), name).await;
    w.join(&w.group, member, shares).await;
    connect_intervals(w.repos(), member, &format!("{athlete}-own-key"), athlete).await;
    member
}

#[tokio::test]
async fn a_coach_reads_an_own_connection_athletes_calendar_through_the_coach_key() {
    let w = World::new().await;
    let owen = own_connection(&w, "Owen Ownkey", "i230", true).await;

    let (payload, refused) = w.coach_reads("Owen").await;
    assert!(!refused, "{payload}");
    assert!(
        read(&payload).contains(&workout(
            "2026-10-01",
            "Hidden from athlete: Coach draft",
            Some("i102")
        )),
        "the coach reads it as their coach, hidden drafts labelled: {payload}"
    );
    let calls = w.api.calendar_calls();
    assert!(!calls.is_empty());
    assert!(
        calls
            .iter()
            .all(|call| call.key == COACH_KEY && call.path.starts_with("/api/v1/athlete/i230/")),
        "the coach's key, at the athlete's own id, never the athlete's key: {calls:?}"
    );
    assert!(
        w.repos()
            .delegated_connections
            .list_live_for_member(owen.id)
            .await
            .unwrap()
            .is_empty(),
        "no link stands behind the read"
    );
    let membership = w
        .repos()
        .groups
        .get_member(&w.group.id.to_string(), owen.id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !membership.coach_consents.calendar_write,
        "a read asks for no calendar-write consent"
    );
}

#[tokio::test]
async fn an_own_connection_athlete_who_stopped_sharing_is_not_read() {
    let w = World::new().await;
    own_connection(&w, "Sam Stopped", "i232", false).await;

    let (refusal, refused) = w.coach_reads("Sam").await;
    assert!(refused, "{refusal}");
    assert!(
        error_text(&refusal).contains("stopped sharing"),
        "{refusal}"
    );
    assert!(w.api.calendar_calls().is_empty(), "{:?}", w.api.hits());
}

#[tokio::test]
async fn a_dead_coach_key_on_an_own_connection_read_flags_the_coach_alone() {
    let w = World::new().await;
    let owen = own_connection(&w, "Owen Ownkey", "i230", true).await;
    w.api.dead.store(true, Ordering::SeqCst);

    let (refusal, refused) = w.coach_reads("Owen").await;
    assert!(refused, "{refusal}");
    assert_eq!(
        w.connection_status(w.coach).await,
        ConnectionStatus::NeedsReauth,
        "the coach's own connection is the one to renew"
    );
    assert_eq!(
        w.connection_status(owen).await,
        ConnectionStatus::Active,
        "never the athlete's own connection"
    );

    w.api.clear();
    let (refusal, refused) = w.coach_reads("Owen").await;
    assert!(refused, "{refusal}");
    assert_eq!(
        reason(&refusal),
        "coach_platform_reconnect_needed",
        "{refusal}"
    );
    assert!(w.api.calendar_calls().is_empty(), "{:?}", w.api.hits());
}
