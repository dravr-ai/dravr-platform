// ABOUTME: A human coach writes a coached athlete's Intervals.icu calendar with athlete= — push, prescribe and withdraw
// ABOUTME: Pins the coach's key at the athlete's path, the ledger under the athlete, each refusal before any write, and the turn budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(all(feature = "provider-intervals-icu", feature = "tools-data"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! A group's human coach names a coached athlete with `athlete=` on
//! `push_training_plan`, `prescribe_workout` or `withdraw_prescribed_workout`
//! and writes that athlete's Intervals.icu calendar through the coach's own
//! API key, at the athlete's path, once the athlete allowed calendar writes
//! on their link and Intervals.icu lists the coach as their coach.
//!
//! Intervals.icu is a loopback stand-in (a test double, per the repo's mock
//! rule: the real provider's wire shape is pinned by
//! `intervals_icu_delegated_read_test` in pierre-providers) that records the
//! method, path and key of every request. Every refusal is asserted with no
//! call to any athlete's events and no live ledger row, and every positive
//! case runs on Dravr's own app (`Transport::WebApp`): an undeclared or
//! external caller writes no one's calendar.

mod common;

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, put};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use common::{create_test_server_resources, create_test_user_with_plan};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use pierre_core::constants::oauth::providers::INTERVALS_ICU;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    ConnectionType, DelegatedConnection, LinkConsents, PrescribedWorkout, RosterAthlete, Tenant,
    TenantId, UserId, UserOAuthToken, API_KEY_TOKEN_TYPE,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_providers::ai_scope::serve_over;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_services::athlete_state::AthleteState;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::endurance_workouts::{
    PrescribeWorkoutTool, WithdrawPrescribedWorkoutTool,
};
use pierre_tool_runtime::implementations::training_plan_push::PushTrainingPlanTool;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use sqlx::Row;
use tokio::net::TcpListener;
use uuid::Uuid;

const COACH_EMAIL: &str = "coach@calendar-write.test";
/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// The coach's API key, the one the stand-in honours.
const COACH_KEY: &str = "coach-key";
/// A key of another Intervals.icu account, which no write may go through.
const OTHER_KEY: &str = "other-account-key";

/// One request the stand-in received.
#[derive(Debug, Clone)]
struct Hit {
    method: &'static str,
    path: String,
    key: String,
}

/// The stand-in's record and what it answers.
#[derive(Clone, Default)]
struct Api {
    hits: Arc<Mutex<Vec<Hit>>>,
    /// The roster entries listed beside the coach's own.
    roster: Arc<Mutex<Vec<Value>>>,
    /// Athletes whose calendar the coach's writes are refused at.
    write_refused: Arc<Mutex<HashSet<String>>>,
    /// Whether the coach's key has stopped working, as a regenerated key
    /// does: every request is then refused as unauthenticated.
    dead: Arc<AtomicBool>,
    next_id: Arc<AtomicI64>,
}

impl Api {
    /// Record a `method` request to `path`, returning whether its key is the
    /// coach's.
    fn record(&self, method: &'static str, path: &str, headers: &HeaderMap) -> bool {
        let key = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Basic "))
            .and_then(|encoded| STANDARD.decode(encoded).ok())
            .and_then(|pair| String::from_utf8(pair).ok())
            .and_then(|pair| pair.strip_prefix("API_KEY:").map(str::to_owned))
            .unwrap_or_default();
        let coach = key == COACH_KEY && !self.dead.load(Ordering::SeqCst);
        self.hits.lock().unwrap().push(Hit {
            method,
            path: path.to_owned(),
            key,
        });
        coach
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().unwrap().clone()
    }

    fn roster_reads(&self) -> usize {
        self.hits()
            .iter()
            .filter(|h| h.path == "/api/v1/athletes")
            .count()
    }

    /// Every call to an athlete's events, read or write.
    fn event_calls(&self) -> Vec<Hit> {
        self.hits()
            .into_iter()
            .filter(|h| h.path.contains("/events"))
            .collect()
    }

    /// The writes to an athlete's events, each asserted to carry the coach's
    /// key.
    fn event_writes(&self) -> Vec<Hit> {
        let hits = self.hits();
        hits.iter()
            .filter(|h| h.path.contains("/events") && h.method != "GET")
            .map(|h| {
                assert_eq!(h.key, COACH_KEY, "{hits:?}");
                h.clone()
            })
            .collect()
    }

    fn refuse_writes(&self, athlete: &str) {
        self.write_refused
            .lock()
            .unwrap()
            .insert(athlete.to_owned());
    }
}

type Answer = (StatusCode, Json<Value>);

fn unauthorized() -> Answer {
    (StatusCode::UNAUTHORIZED, Json(json!({})))
}

async fn athletes_route(State(api): State<Api>, headers: HeaderMap) -> Answer {
    if !api.record("GET", "/api/v1/athletes", &headers) {
        return unauthorized();
    }
    // The coach's own entry binds the key to the coach by email.
    let mut roster = vec![json!({ "id": COACH_ATHLETE, "name": "Casey", "email": COACH_EMAIL })];
    roster.extend(api.roster.lock().unwrap().iter().cloned());
    (StatusCode::OK, Json(Value::Array(roster)))
}

async fn list_events(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> Answer {
    if !api.record(
        "GET",
        &format!("/api/v1/athlete/{athlete}/events"),
        &headers,
    ) {
        return unauthorized();
    }
    (StatusCode::OK, Json(json!([])))
}

async fn create_event(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
) -> Answer {
    if !api.record(
        "POST",
        &format!("/api/v1/athlete/{athlete}/events"),
        &headers,
    ) {
        return unauthorized();
    }
    if api.write_refused.lock().unwrap().contains(&athlete) {
        return (StatusCode::FORBIDDEN, Json(json!({})));
    }
    let id = api.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    (StatusCode::OK, Json(json!({ "id": id })))
}

async fn update_event(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((athlete, event)): Path<(String, String)>,
) -> Answer {
    if !api.record(
        "PUT",
        &format!("/api/v1/athlete/{athlete}/events/{event}"),
        &headers,
    ) {
        return unauthorized();
    }
    (StatusCode::OK, Json(json!({})))
}

async fn delete_events(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
    Json(doomed): Json<Vec<Value>>,
) -> Answer {
    if !api.record(
        "PUT",
        &format!("/api/v1/athlete/{athlete}/events/bulk-delete"),
        &headers,
    ) {
        return unauthorized();
    }
    (
        StatusCode::OK,
        Json(json!({ "eventsDeleted": doomed.len() })),
    )
}

async fn spawn_api(api: Api) -> String {
    let app = Router::new()
        .route("/api/v1/athletes", get(athletes_route))
        .route(
            "/api/v1/athlete/{athlete}/events",
            get(list_events).post(create_event),
        )
        .route(
            "/api/v1/athlete/{athlete}/events/bulk-delete",
            put(delete_events),
        )
        .route(
            "/api/v1/athlete/{athlete}/events/{event}",
            put(update_event),
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

/// `who`'s own Intervals.icu API key in `tenant`, for account `athlete`,
/// with the connection a linked key registers.
async fn connect_intervals(
    repos: &RepositoryRegistry,
    who: Person,
    tenant: TenantId,
    key: &str,
    athlete: &str,
) {
    let now = Utc::now();
    repos
        .oauth_tokens
        .upsert_token(&UserOAuthToken {
            id: Uuid::new_v4().to_string(),
            user_id: who.id,
            tenant_id: tenant.to_string(),
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
        .register_connection(who.id, tenant, INTERVALS_ICU, &ConnectionType::Manual, None)
        .await
        .unwrap();
}

/// The Monday at least a week out, so every plan date is in the future on
/// whatever day the suite runs.
fn next_plan_monday() -> NaiveDate {
    let mut day = Utc::now().date_naive() + Duration::days(7);
    while day.weekday() != Weekday::Mon {
        day += Duration::days(1);
    }
    day
}

fn planned(date: NaiveDate, workout: &str, minutes: u32) -> PlannedDay {
    PlannedDay {
        date: date.format("%Y-%m-%d").to_string(),
        sport: "run".to_owned(),
        workout: workout.to_owned(),
        duration_min: Some(minutes),
        intensity: "Z2".to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
        template_source: None,
    }
}

/// A structured session the coach authored in conversation.
fn tempo_session() -> Value {
    json!({
        "name": "Coach tempo 40",
        "sport": "run",
        "intensity_distribution": "threshold",
        "structure": [
            { "label": "Warm-up", "duration_seconds": 600, "target_zone": "Z1" },
            { "label": "Tempo", "duration_seconds": 1200, "target_zone": "Z3" },
            { "label": "Cool-down", "duration_seconds": 600, "target_zone": "Z1" }
        ]
    })
}

impl World {
    /// A server whose Intervals.icu calls go to the stand-in; a group the
    /// coach coaches; the coach's own Intervals.icu key. The roster cache
    /// starts cold.
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

        let owner = person(&res, "owner@calendar-write.test", "Olive Owner").await;
        let coach = person(&res, COACH_EMAIL, "Casey Coach").await;
        let repos = &res.common.repos;
        connect_intervals(repos, coach, coach.tenant, COACH_KEY, COACH_ATHLETE).await;
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

    /// `who` joins the coach's group, sharing with its coach or not.
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
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// A member of the coach's group named `name`, on the roster as
    /// `athlete` with the wire `icu_permission` value `access` (`None`
    /// leaves it out), linked and confirmed with `calendar_write`.
    async fn athlete(
        &self,
        name: &str,
        athlete: &str,
        access: Option<Value>,
        calendar_write: bool,
    ) -> (Person, DelegatedConnection) {
        let email = format!("{athlete}@calendar-write.test");
        let member = person(&self.res, &email, name).await;
        self.join(&self.group, member, true).await;
        let mut entry = json!({ "id": athlete, "name": "Roster Name", "email": email });
        if let Some(access) = access {
            entry["icu_permission"] = access;
        }
        self.api.roster.lock().unwrap().push(entry);
        let link = self
            .link(self.coach, &self.group, member, athlete, calendar_write)
            .await;
        (member, link)
    }

    /// A confirmed Intervals.icu link of `coach`'s in `group` for roster
    /// athlete `athlete` as `member`, with the connection it registers.
    async fn link(
        &self,
        coach: Person,
        group: &CoachingGroup,
        member: Person,
        athlete: &str,
        calendar_write: bool,
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
                RosterAthlete::new(
                    athlete,
                    None,
                    Some(format!("{athlete}@calendar-write.test")),
                ),
            ))
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet");
        let link = repos
            .delegated_connections
            .confirm(
                proposed.id,
                member.id,
                member.tenant,
                LinkConsents {
                    calendar_write,
                    wellness_read: false,
                },
                Utc::now(),
            )
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

    /// `athlete`'s active plan: two sessions from next week on, derived from
    /// data stamped `policy`.
    async fn plan(&self, athlete: Person, policy: TransportPolicy) {
        let monday = next_plan_monday();
        let goal = GoalRace {
            name: "Fall race".to_owned(),
            date: (monday + Duration::days(70)).format("%Y-%m-%d").to_string(),
            discipline: "trail".to_owned(),
            priority: RacePriority::A,
        };
        let phases = vec![PlanPhase {
            kind: PhaseKind::Base,
            start: monday.format("%Y-%m-%d").to_string(),
            weeks: 10,
            intent: "rebuild volume".to_owned(),
            target_hours: None,
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        }];
        let days = vec![
            planned(monday, "Easy 45", 45),
            planned(monday + Duration::days(2), "Steady 60", 60),
        ];
        let start = monday.format("%Y-%m-%d").to_string();
        self.repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &athlete.tenant.to_string(),
                user_id: &athlete.id.to_string(),
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "steady base",
                    phases: &phases,
                    source_conversation_id: None,
                    flavour: None,
                    season_start: None,
                    season_end: None,
                }),
                weeks: &[PlanWeekInput {
                    week_start: &start,
                    focus: "base",
                    days: &days,
                    adjustment_reason: "",
                    phase_index: None,
                }],
                transport_policy: policy,
            })
            .await
            .unwrap();
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

    /// Call `tool` as `who` in `tenant` with `args`: its payload, and whether
    /// it refused.
    async fn call<T: McpTool<dyn ToolRuntime>>(
        &self,
        tool: &T,
        who: Person,
        tenant: TenantId,
        args: Value,
    ) -> (Value, bool) {
        let runtime: Arc<dyn ToolRuntime> = Arc::clone(&self.res) as Arc<dyn ToolRuntime>;
        let ctx = ToolContext::new()
            .with_user(who.id.to_string())
            .with_tenant(tenant.to_string())
            .with_auth_method("jwt_bearer");
        let response = tool.execute(&runtime, &ctx, args).await;
        (
            response
                .structured_content
                .expect("tool result carries structured content"),
            response.is_error,
        )
    }

    /// `tool` as the coach, from Dravr's web app, outside any conversation.
    async fn in_app<T: McpTool<dyn ToolRuntime>>(&self, tool: &T, args: Value) -> (Value, bool) {
        serve_over(
            Transport::WebApp,
            self.call(tool, self.coach, self.coach.tenant, args),
        )
        .await
    }

    /// `tool` as the coach, from their direct chat in Dravr's web app.
    async fn in_dm<T: McpTool<dyn ToolRuntime>>(&self, tool: &T, args: Value) -> (Value, bool) {
        let conversation = self.coach_dm().await;
        CONVERSATION_ID
            .scope(Some(conversation), self.in_app(tool, args))
            .await
    }

    /// The rows live on `who`'s Intervals.icu calendar, by the ledger.
    async fn live(&self, who: Person) -> Vec<PrescribedWorkout> {
        self.repos()
            .prescribed_workouts
            .list_live_calendar_events(who.tenant, who.id, INTERVALS_ICU, None)
            .await
            .unwrap()
    }

    /// `tool` as `who`, in their own tenant, from Dravr's web app: the
    /// athlete's own call.
    async fn as_athlete<T: McpTool<dyn ToolRuntime>>(
        &self,
        tool: &T,
        who: Person,
        args: Value,
    ) -> (Value, bool) {
        serve_over(Transport::WebApp, self.call(tool, who, who.tenant, args)).await
    }

    /// How many ledger rows of any status `who` holds. The repository reads
    /// a row by id or the live rows of one provider, never the whole ledger,
    /// so the rows are counted on the table.
    async fn ledger_rows(&self, who: Person) -> i64 {
        const SQL: &str =
            "SELECT COUNT(*) AS n FROM prescribed_workouts WHERE tenant_id = $1 AND user_id = $2";
        match self.res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => sqlx::query(SQL)
                .bind(who.tenant)
                .bind(UserId::from_uuid(who.id))
                .fetch_one(db.pool())
                .await
                .unwrap()
                .get::<i64, _>("n"),
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => sqlx::query(SQL)
                .bind(who.tenant)
                .bind(UserId::from_uuid(who.id))
                .fetch_one(db.pool())
                .await
                .unwrap()
                .get::<i64, _>("n"),
        }
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

fn reason(payload: &Value) -> &str {
    payload["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a named refusal, got {payload}"))
}

/// An athlete's own write to the calendar their human coach's link holds,
/// refused in that coach's terms and never with a request to connect one.
fn assert_human_coach_writes(payload: &Value, refused: bool) {
    assert!(refused, "{payload}");
    assert_eq!(reason(payload), "human_coach_writes_calendar", "{payload}");
    let text = error_text(payload);
    assert!(text.contains("human coach's link"), "{payload}");
    // The coach writes it only with the athlete's calendar_write consent,
    // which the refusal names so the athlete can give it.
    assert!(text.contains("allow calendar writes"), "{payload}");
    assert!(!text.contains("Connect one of"), "{payload}");
    assert!(payload.get("details").is_none(), "{payload}");
}

fn error_text(payload: &Value) -> &str {
    payload["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal, got {payload}"))
}

// ════════════════════════════════════════════════════════════════════════
// Writes
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_coach_pushes_a_consenting_athletes_plan_with_the_coach_key_at_the_athletes_path() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::AnyTransport).await;

    let (report, refused) = w
        .in_dm(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert!(!refused, "{report}");
    assert_eq!(report["failed"], json!([]), "{report}");
    let created = report["created"].as_u64().unwrap();
    // Two sessions and the week's note.
    assert_eq!(created, 3, "{report}");

    let writes = w.api.event_writes();
    assert_eq!(writes.len(), 3, "{writes:?}");
    assert!(writes
        .iter()
        .all(|h| h.method == "POST" && h.path == "/api/v1/athlete/i201/events"));
    assert_eq!(w.api.roster_reads(), 1, "the access check read the roster");

    // The ledger is the athlete's: their rows, each holding the event the
    // calendar issued; nothing is filed under the coach.
    let live = w.live(alex).await;
    assert_eq!(live.len(), 3);
    assert!(live.iter().all(|row| row.provider_event_id.is_some()));
    assert!(w.live(w.coach).await.is_empty());
    assert!(
        report.get("unpublished_draft").is_none(),
        "no draft, nothing to say: {report}"
    );
}

/// A coach's unpublished draft never reaches the athlete's calendar: the push
/// carries the published plan, and says the draft is still the coach's
/// (carnet#1069).
#[tokio::test]
async fn a_coachs_push_carries_the_published_plan_never_their_draft() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::AnyTransport).await;
    let monday = next_plan_monday();
    let date = |offset: i64| {
        (monday + Duration::days(offset))
            .format("%Y-%m-%d")
            .to_string()
    };
    let (saved, refused) = w
        .in_dm(
            &SaveTrainingPlanTool,
            json!({
                "athlete": "Alex",
                "weeks": [{
                    "week_start": date(0),
                    "focus": "the coach's draft",
                    "days": [{ "date": date(1), "sport": "run", "workout": "Draft tempo", "duration_min": 50, "intensity": "Z3" }],
                }],
            }),
        )
        .await;
    assert!(!refused, "{saved}");
    assert_eq!(saved["draft"], true, "{saved}");

    let (report, refused) = w
        .in_dm(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert!(!refused, "{report}");
    assert!(
        report["unpublished_draft"]
            .as_str()
            .is_some_and(|note| note.contains("only the published plan is pushed")),
        "{report}"
    );
    // The published plan's two sessions and its week note went out; the
    // draft's day, a Tuesday the published plan leaves empty, did not.
    assert_eq!(report["created"], 3, "{report}");
    let drafted_day = monday + Duration::days(1);
    assert!(
        w.live(alex)
            .await
            .iter()
            .all(|row| row.prescribed_for_date != drafted_day
                && !row.payload_json.contains("Draft tempo")),
        "the draft never reaches the calendar"
    );
}

#[tokio::test]
async fn a_coach_prescribes_and_withdraws_on_a_consenting_athletes_calendar() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    let date = (Utc::now().date_naive() + Duration::days(3))
        .format("%Y-%m-%d")
        .to_string();

    let (prescribed, refused) = w
        .in_dm(
            &PrescribeWorkoutTool,
            json!({ "athlete": "Alex", "date": date, "session": tempo_session() }),
        )
        .await;
    assert!(!refused, "{prescribed}");
    let id: Uuid = prescribed["prescription_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let repos = w.repos();
    let row = repos
        .prescribed_workouts
        .get_prescribed_workout(alex.tenant, alex.id, id)
        .await
        .unwrap()
        .expect("the ledger row is the athlete's");
    assert_eq!(row.status, PrescribedWorkout::STATUS_PUSHED);
    assert_eq!(row.agent_id.as_deref(), Some(w.coach_agent.as_str()));
    // The session the coach authored is stored as the athlete's, never as
    // the coach's.
    let slug = prescribed["template_slug"].as_str().unwrap();
    assert!(repos
        .workout_templates
        .get_user_workout_template(alex.tenant, alex.id, slug)
        .await
        .unwrap()
        .is_some());
    assert!(repos
        .workout_templates
        .get_user_workout_template(w.coach.tenant, w.coach.id, slug)
        .await
        .unwrap()
        .is_none());

    let (withdrawn, refused) = w
        .in_dm(
            &WithdrawPrescribedWorkoutTool,
            json!({ "athlete": "Alex", "prescription_id": id.to_string() }),
        )
        .await;
    assert!(!refused, "{withdrawn}");
    let row = repos
        .prescribed_workouts
        .get_prescribed_workout(alex.tenant, alex.id, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, PrescribedWorkout::STATUS_WITHDRAWN);

    let lines: Vec<String> = w
        .api
        .event_writes()
        .iter()
        .map(|h| format!("{} {}", h.method, h.path))
        .collect();
    assert_eq!(
        lines,
        [
            "POST /api/v1/athlete/i201/events",
            "PUT /api/v1/athlete/i201/events/bulk-delete",
        ]
    );
}

#[tokio::test]
async fn the_key_that_writes_is_the_links_never_one_in_the_coachs_request_tenant() {
    let w = World::new().await;
    let (alex, link) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::AnyTransport).await;
    // The coach works from a second tenant, which holds a key of another
    // Intervals.icu account.
    let now = Utc::now();
    let elsewhere = TenantId::generate();
    w.repos()
        .tenants
        .create(&Tenant {
            id: elsewhere,
            name: "Coach's other workspace".to_owned(),
            slug: format!("elsewhere-{elsewhere}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: w.coach.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    connect_intervals(w.repos(), w.coach, elsewhere, OTHER_KEY, "i300").await;
    assert_ne!(link.coach_tenant_id, elsewhere);

    let (report, refused) = serve_over(
        Transport::WebApp,
        w.call(
            &PushTrainingPlanTool,
            w.coach,
            elsewhere,
            json!({ "athlete": "Alex" }),
        ),
    )
    .await;
    assert!(!refused, "{report}");
    assert_eq!(report["created"], 3, "{report}");
    let hits = w.api.hits();
    assert!(
        hits.iter().all(|h| h.key == COACH_KEY),
        "only the key the link was proposed under is used: {hits:?}"
    );
    assert_eq!(w.live(alex).await.len(), 3);
}

#[tokio::test]
async fn a_plan_from_first_party_only_data_is_pushed_from_the_app_never_over_mcp() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::FirstPartyOnly).await;

    let (refusal, refused) = serve_over(
        Transport::McpHttp,
        w.call(
            &PushTrainingPlanTool,
            w.coach,
            w.coach.tenant,
            json!({ "athlete": "Alex" }),
        ),
    )
    .await;
    assert!(refused);
    assert_eq!(reason(&refusal), "external_transport", "{refusal}");
    assert!(w.api.hits().is_empty(), "nothing was read or written");

    // The athlete granted their human coach calendar writes, which is the
    // opt-in their data's terms ask for (carnet#770).
    let (report, refused) = w
        .in_app(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert!(!refused, "{report}");
    let live = w.live(alex).await;
    assert_eq!(live.len(), 3);
    assert!(live
        .iter()
        .all(|row| row.transport_policy == TransportPolicy::FirstPartyOnly));
}

// ════════════════════════════════════════════════════════════════════════
// Refusals
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn every_refusal_is_named_and_reaches_no_athletes_calendar() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    let (nora, _) = w
        .athlete("Nora Noconsent", "i211", Some(json!("WRITE")), false)
        .await;
    let (fiona, _) = w
        .athlete("Fiona Follower", "i212", Some(json!("READ")), true)
        .await;
    let (nico, _) = w
        .athlete("Nico Noaccess", "i213", Some(json!("NONE")), true)
        .await;
    let (uma, _) = w
        .athlete("Uma Unknown", "i214", Some(Value::Null), true)
        .await;
    // An athlete who connects Intervals.icu themselves holds no link.
    let owen = person(&w.res, "owen@calendar-write.test", "Owen Ownkey").await;
    w.join(&w.group, owen, true).await;
    connect_intervals(w.repos(), owen, owen.tenant, "owen-key", "i230").await;
    // An athlete who stopped sharing with the coach.
    let sara = person(&w.res, "sara@calendar-write.test", "Sara Stopped").await;
    w.join(&w.group, sara, false).await;
    // An athlete linked through another coach, who is also in this coach's
    // group.
    let blake = person(&w.res, "blake@calendar-write.test", "Blake Othercoach").await;
    connect_intervals(w.repos(), blake, blake.tenant, "blake-key", "i400").await;
    let other_group = coached_group(&w.res, w.owner, blake, "Other Squad").await;
    let bea = person(&w.res, "i216@calendar-write.test", "Bea Elsewhere").await;
    w.join(&other_group, bea, true).await;
    w.join(&w.group, bea, true).await;
    w.link(blake, &other_group, bea, "i216", true).await;

    let push = |athlete: &str| json!({ "athlete": athlete });
    let prescribe = |athlete: &str| json!({ "athlete": athlete, "date": "2030-01-07", "session": tempo_session() });

    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Nora")).await;
    assert_eq!(reason(&refusal), "no_calendar_write_consent", "{refusal}");
    let (refusal, _) = w.in_app(&PrescribeWorkoutTool, prescribe("Nora")).await;
    assert_eq!(reason(&refusal), "no_calendar_write_consent", "{refusal}");
    assert_eq!(
        w.api.roster_reads(),
        0,
        "nothing provider-side is read before the athlete's consent is known"
    );

    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Fiona")).await;
    assert_eq!(reason(&refusal), "coach_follower", "{refusal}");
    assert!(error_text(&refusal).contains("follow this athlete"));
    assert_eq!(w.api.roster_reads(), 1);
    let (refusal, _) = w.in_app(&PrescribeWorkoutTool, prescribe("Fiona")).await;
    assert_eq!(reason(&refusal), "coach_follower", "{refusal}");

    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Nico")).await;
    assert_eq!(reason(&refusal), "athlete_no_access", "{refusal}");
    assert_eq!(w.api.roster_reads(), 1, "the cached roster answers");

    // Access the platform did not report is read once more, live, and is
    // never taken for a follower's.
    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Uma")).await;
    assert_eq!(reason(&refusal), "coach_access_unknown", "{refusal}");
    assert!(!error_text(&refusal).contains("follow"), "{refusal}");
    assert_eq!(w.api.roster_reads(), 2);

    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Owen")).await;
    assert_eq!(reason(&refusal), "own_connection", "{refusal}");

    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Sara")).await;
    assert!(
        error_text(&refusal).contains("stopped sharing"),
        "{refusal}"
    );

    let (refusal, _) = w.in_app(&PushTrainingPlanTool, push("Bea")).await;
    assert_eq!(
        reason(&refusal),
        "linked_through_another_coach",
        "{refusal}"
    );

    // A group's room is read by every member.
    let (refusal, _) = CONVERSATION_ID
        .scope(
            Some("a-room-the-coachs-tenant-cannot-resolve".to_owned()),
            w.in_app(&PushTrainingPlanTool, push("Alex")),
        )
        .await;
    assert_eq!(reason(&refusal), "group_room", "{refusal}");

    // An external caller writes no one's calendar.
    for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
        let (refusal, _) = serve_over(
            transport,
            w.call(
                &PrescribeWorkoutTool,
                w.coach,
                w.coach.tenant,
                prescribe("Alex"),
            ),
        )
        .await;
        assert_eq!(reason(&refusal), "external_transport", "{refusal}");
    }

    assert!(
        w.api.event_calls().is_empty(),
        "no refusal reached a calendar: {:?}",
        w.api.event_calls()
    );
    for athlete in [alex, nora, fiona, nico, uma, owen, sara, bea] {
        assert!(w.live(athlete).await.is_empty());
        // A refused prescription saves nothing among the athlete's sessions.
        let saved = w
            .repos()
            .workout_templates
            .get_user_workout_template(athlete.tenant, athlete.id, "coach_tempo_40")
            .await
            .unwrap();
        assert!(saved.is_none(), "{saved:?}");
    }
}

#[tokio::test]
async fn an_athlete_off_the_coachs_roster_is_refused_and_the_live_read_ends_the_link() {
    let w = World::new().await;
    // Linked, consenting, and no longer listed on the coach's roster.
    let (gone, _) = w
        .athlete("Gale Gone", "i240", Some(json!("WRITE")), true)
        .await;
    w.api.roster.lock().unwrap().clear();

    let (refusal, refused) = w
        .in_app(&PushTrainingPlanTool, json!({ "athlete": "Gale" }))
        .await;
    assert!(refused);
    assert_eq!(reason(&refusal), "athlete_not_on_roster", "{refusal}");
    assert!(w.api.event_calls().is_empty());
    assert!(
        !w.linked(gone).await,
        "the live roster read ends a link whose athlete it no longer lists"
    );
}

#[tokio::test]
async fn a_write_the_platform_refuses_is_named_and_the_link_survives() {
    let w = World::new().await;
    let (wren, _) = w
        .athlete("Wren Refused", "i220", Some(json!("WRITE")), true)
        .await;
    w.plan(wren, TransportPolicy::AnyTransport).await;
    // The roster read says Coach; Intervals.icu still refuses the write.
    w.api.refuse_writes("i220");

    let (refusal, refused) = w
        .in_app(
            &PrescribeWorkoutTool,
            json!({ "athlete": "Wren", "date": "2030-01-07", "session": tempo_session() }),
        )
        .await;
    assert!(refused);
    assert!(
        error_text(&refusal).contains("refused this calendar write"),
        "{refusal}"
    );

    let (report, refused) = w
        .in_app(&PushTrainingPlanTool, json!({ "athlete": "Wren" }))
        .await;
    assert!(!refused, "per-entry failures are reported: {report}");
    let failed = report["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 3, "{report}");
    assert!(failed.iter().all(|entry| entry["error"]
        .as_str()
        .is_some_and(|e| e.contains("refused this calendar write"))));

    assert!(w.live(wren).await.is_empty());
    assert!(w.linked(wren).await, "a refused write ends nothing");
}

#[tokio::test]
async fn a_dead_coach_key_on_a_write_flags_the_coachs_connection_and_refuses_the_next_first() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    let prescribe =
        || json!({ "athlete": "Alex", "date": "2030-01-07", "session": tempo_session() });
    let (first, refused) = w.in_app(&PrescribeWorkoutTool, prescribe()).await;
    assert!(!refused, "{first}");

    // The coach regenerates their key on Intervals.icu: the one Dravr holds
    // stops working. The roster is still cached, so the write is what finds
    // it dead.
    w.api.dead.store(true, Ordering::SeqCst);
    let (refusal, refused) = w.in_app(&PrescribeWorkoutTool, prescribe()).await;
    assert!(refused);
    assert!(
        error_text(&refusal).contains("the coach needs to reconnect"),
        "{refusal}"
    );
    let connection = w
        .repos()
        .provider_connections
        .get_for_user(w.coach.id, Some(w.coach.tenant))
        .await
        .unwrap()
        .into_iter()
        .find(|c| c.provider == INTERVALS_ICU)
        .expect("the coach's own connection");
    assert!(
        connection.status.requires_reauth(),
        "the coach's connection is flagged: {:?}",
        connection.status
    );

    // The next write is refused before any request, in the coach's words.
    let calls = w.api.event_calls().len();
    let (refusal, refused) = w.in_app(&PrescribeWorkoutTool, prescribe()).await;
    assert!(refused);
    assert_eq!(
        reason(&refusal),
        "coach_platform_reconnect_needed",
        "{refusal}"
    );
    assert_eq!(w.api.event_calls().len(), calls);
    assert!(
        w.linked(alex).await,
        "the member's link stands: only the coach can renew the key"
    );
}

#[tokio::test]
async fn a_second_push_in_one_turn_is_held_by_the_guardian_budget() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    let (ben, _) = w
        .athlete("Ben Second", "i202", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::AnyTransport).await;
    w.plan(ben, TransportPolicy::AnyTransport).await;

    let runtime: Arc<dyn ToolRuntime> = w.res.clone();
    let executor = UniversalToolExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .with_turn_token("coach-turn".to_owned())
        .with_transport(Transport::WebApp);
    let push = |athlete: &str| UniversalRequest {
        tool_name: "push_training_plan".to_owned(),
        parameters: json!({ "athlete": athlete }),
        user_id: w.coach.id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(w.coach.tenant.to_string()),
    };

    let first = executor.execute_tool(push("Alex")).await.unwrap();
    assert!(first.success, "{:?}", first.error);
    let second = executor.execute_tool(push("Ben")).await.unwrap();
    assert!(!second.success);
    let result = second.result.expect("the denial is in-band");
    assert_eq!(result["error_code"], "guardian_denied", "{result}");
    assert_eq!(result["reason"], "budget_exceeded", "{result}");

    assert_eq!(w.live(alex).await.len(), 3);
    assert!(w.live(ben).await.is_empty());
    assert!(w
        .api
        .event_calls()
        .iter()
        .all(|h| h.path.starts_with("/api/v1/athlete/i201/")));
}

#[tokio::test]
async fn a_calendar_read_only_through_a_coachs_link_is_not_the_athletes_to_push() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::AnyTransport).await;

    let (state, _) = AthleteState::read(w.repos(), alex.tenant, alex.id, Utc::now())
        .await
        .unwrap();
    assert!(
        !state.calendar_writable,
        "the athlete's only Intervals.icu calendar takes their human coach's writes"
    );

    // Named or not, the athlete's own push and prescription are refused in
    // their human coach's terms before anything is read: never by asking
    // them to connect a calendar, which would end the link.
    let session = |extra: Value| {
        let mut args = json!({ "date": "2030-01-07", "session": tempo_session() });
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        args
    };
    let (refusal, refused) = w.as_athlete(&PushTrainingPlanTool, alex, json!({})).await;
    assert_human_coach_writes(&refusal, refused);
    let (refusal, refused) = w
        .as_athlete(
            &PushTrainingPlanTool,
            alex,
            json!({ "provider": "intervals_icu" }),
        )
        .await;
    assert_human_coach_writes(&refusal, refused);
    let (refusal, refused) = w
        .as_athlete(
            &PrescribeWorkoutTool,
            alex,
            session(json!({ "provider": "intervals_icu" })),
        )
        .await;
    assert_human_coach_writes(&refusal, refused);
    assert!(
        w.api.hits().is_empty(),
        "nothing was read through the coach's key: {:?}",
        w.api.hits()
    );
    assert_eq!(w.ledger_rows(alex).await, 0, "not even a failed row");

    // The coach's own entry on the athlete's calendar is replaced or
    // withdrawn by the coach alone.
    let (prescribed, refused) = w
        .in_dm(&PrescribeWorkoutTool, session(json!({ "athlete": "Alex" })))
        .await;
    assert!(!refused, "{prescribed}");
    let coach_entry = prescribed["prescription_id"].as_str().unwrap().to_owned();
    let hits = w.api.hits().len();
    let (refusal, refused) = w
        .as_athlete(
            &PrescribeWorkoutTool,
            alex,
            session(json!({ "replaces": coach_entry })),
        )
        .await;
    assert_human_coach_writes(&refusal, refused);
    let (refusal, refused) = w
        .as_athlete(
            &WithdrawPrescribedWorkoutTool,
            alex,
            json!({ "prescription_id": coach_entry }),
        )
        .await;
    assert_human_coach_writes(&refusal, refused);
    assert_eq!(w.api.hits().len(), hits, "{:?}", w.api.hits());
    assert_eq!(w.ledger_rows(alex).await, 1, "the coach's entry alone");
    let live = w.live(alex).await;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id.to_string(), coach_entry);
}

#[tokio::test]
async fn an_athlete_reading_through_the_link_is_told_their_human_coach_pushes_it() {
    let w = World::new().await;
    let (alex, _) = w
        .athlete("Alex Athlete", "i201", Some(json!("WRITE")), true)
        .await;
    w.plan(alex, TransportPolicy::AnyTransport).await;
    let (report, refused) = w
        .in_dm(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert!(!refused, "{report}");

    let (plan, refused) = w.as_athlete(&GetTrainingPlanTool, alex, json!({})).await;
    assert!(!refused, "{plan}");
    assert_eq!(plan["calendar"]["provider"], INTERVALS_ICU, "{plan}");
    assert_eq!(plan["calendar"]["pushed_by"], "human_coach", "{plan}");

    // The coach reading for the athlete pushes it themselves.
    let (coach_view, refused) = w
        .in_dm(&GetTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert!(!refused, "{coach_view}");
    assert!(
        coach_view["calendar"].get("pushed_by").is_none(),
        "{coach_view}"
    );

    // The athlete's own change leaves the calendar behind, and says who
    // brings it up to date.
    let monday = next_plan_monday();
    let day = |offset: i64, workout: &str, minutes: u32| {
        json!({
            "date": (monday + Duration::days(offset)).format("%Y-%m-%d").to_string(),
            "sport": "run",
            "workout": workout,
            "duration_min": minutes,
            "intensity": "Z2",
        })
    };
    let (saved, refused) = w
        .as_athlete(
            &SaveTrainingPlanTool,
            alex,
            json!({ "weeks": [{
                "week_start": monday.format("%Y-%m-%d").to_string(),
                "focus": "base",
                "days": [day(0, "Easy 50", 50), day(2, "Steady 60", 60)],
            }] }),
        )
        .await;
    assert!(!refused, "{saved}");
    assert_eq!(saved["calendar"]["stale"], true, "{saved}");
    assert_eq!(saved["calendar"]["pushed_by"], "human_coach", "{saved}");
}
