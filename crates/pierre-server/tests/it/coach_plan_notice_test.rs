// ABOUTME: An athlete is told when their human coach publishes the coach's draft of their plan, or pushes it onto their calendar
// ABOUTME: One notice per real publish or writing coach push, in the athlete's tenant; drafts, discards, conflicts and own pushes tell no one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(all(
    feature = "client-notifications",
    feature = "provider-intervals-icu",
    feature = "tools-data"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1069. A group's human coach builds a coached athlete's plan in a
//! draft the athlete never sees, publishes it, and can push it onto the
//! athlete's Intervals.icu calendar through the coach's link. Each of those
//! two acts changes what the athlete reads without the athlete doing
//! anything, so each tells them: `coach_plan_published` when the draft goes
//! live — from `publish_training_plan` or the thread's Publish button, which
//! share one entry point — and `coach_plan_pushed` when the coach's push put
//! workouts on their calendar.
//!
//! The notices are read back from the athlete's feed in the athlete's own
//! tenant, rendered in their language (French, the default). Everything else
//! — a draft save, a discard, a refused publish, a publish with nothing to
//! publish, a push that wrote nothing or that Intervals.icu refused, and the
//! athlete's own push — leaves the feed as it was, and the coach is never
//! told anything. Each negative is checked after a later notice has landed
//! where the scenario has one, so a stray dispatch has had its time.
//!
//! Intervals.icu is a loopback stand-in (a test double, per the repo's mock
//! rule: the real wire shape is pinned by the provider's own tests) that keeps
//! the events written to it, so a second push reads them back as current.

use crate::common;
use crate::helpers;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, put};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::INTERVALS_ICU;
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    ConnectionType, DelegatedConnection, LinkConsents, RosterAthlete, TenantId, UserOAuthToken,
    API_KEY_TOKEN_TYPE,
};
use pierre_core::transport::{Transport, TransportPolicy};
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use pierre_notifications::models::{
    Notification, NotificationCategory, TransportPolicy as PushTransportPolicy,
};
use pierre_notifications::TenantId as NoticeTenantId;
use pierre_providers::ai_scope::serve_over;
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plan_drafts::PublishTrainingPlanTool;
use pierre_tool_runtime::implementations::training_plan_push::PushTrainingPlanTool;
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::time::sleep;
use uuid::Uuid;

const COACH_EMAIL: &str = "coach@plan-notice.test";
const COACH_NAME: &str = "Casey Coach";
/// The coach's own Intervals.icu athlete id.
const COACH_ATHLETE: &str = "i100";
/// The coach's API key.
const COACH_KEY: &str = "coach-key";
/// A self-coached athlete's own API key.
const SAM_KEY: &str = "sam-key";
/// The race the athlete's season is laid toward.
const RACE: &str = "Fall race";
const PUBLISHED: &str = "coach_plan_published";
const PUSHED: &str = "coach_plan_pushed";

// ════════════════════════════════════════════════════════════════════════
// The Intervals.icu stand-in
// ════════════════════════════════════════════════════════════════════════

/// What the stand-in holds: the coach's roster, the athletes whose calendar
/// refuses writes, and the events written to each athlete's calendar.
#[derive(Clone, Default)]
struct Api {
    roster: Arc<Mutex<Vec<Value>>>,
    refused: Arc<Mutex<HashSet<String>>>,
    events: Arc<Mutex<HashMap<String, Vec<Value>>>>,
    next_id: Arc<AtomicI64>,
}

impl Api {
    fn refuse_writes(&self, athlete: &str, refused: bool) {
        let mut set = self.refused.lock().unwrap();
        if refused {
            set.insert(athlete.to_owned());
        } else {
            set.remove(athlete);
        }
    }
}

type Answer = (StatusCode, Json<Value>);

/// The API key a request carries, from its Basic `API_KEY:<key>` header.
fn api_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Basic ")
        .and_then(|encoded| STANDARD.decode(encoded).ok())
        .and_then(|pair| String::from_utf8(pair).ok())
        .and_then(|pair| pair.strip_prefix("API_KEY:").map(str::to_owned))
}

/// Whether the request carries a key the stand-in knows.
fn admitted(headers: &HeaderMap) -> bool {
    api_key(headers).is_some_and(|key| key == COACH_KEY || key == SAM_KEY)
}

fn unauthorized() -> Answer {
    (StatusCode::UNAUTHORIZED, Json(json!({})))
}

async fn athletes_route(State(api): State<Api>, headers: HeaderMap) -> Answer {
    if api_key(&headers).as_deref() != Some(COACH_KEY) {
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
    if !admitted(&headers) {
        return unauthorized();
    }
    let events = api
        .events
        .lock()
        .unwrap()
        .get(&athlete)
        .cloned()
        .unwrap_or_default();
    (StatusCode::OK, Json(Value::Array(events)))
}

async fn create_event(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(athlete): Path<String>,
    Json(body): Json<Value>,
) -> Answer {
    if !admitted(&headers) {
        return unauthorized();
    }
    if api.refused.lock().unwrap().contains(&athlete) {
        return (StatusCode::FORBIDDEN, Json(json!({})));
    }
    let id = api.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    api.events
        .lock()
        .unwrap()
        .entry(athlete)
        .or_default()
        .push(json!({
            "id": id,
            "start_date_local": body["start_date_local"],
            "external_id": body["external_id"],
        }));
    (StatusCode::OK, Json(json!({ "id": id })))
}

async fn update_event(headers: HeaderMap) -> Answer {
    if !admitted(&headers) {
        return unauthorized();
    }
    (StatusCode::OK, Json(json!({})))
}

async fn delete_events(headers: HeaderMap, Json(doomed): Json<Vec<Value>>) -> Answer {
    if !admitted(&headers) {
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

// ════════════════════════════════════════════════════════════════════════
// The world: a group, its human coach, and linked athletes
// ════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

struct World {
    res: Arc<ServerContext>,
    api: Api,
    coach: Person,
    group: CoachingGroup,
    /// The agent the coach's direct chat answers as.
    coach_agent: String,
}

/// A verified user whose email is `email` and display name `name`, in a
/// tenant of their own.
async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

/// `who`'s own Intervals.icu API key for account `athlete`, with the
/// connection a linked key registers.
async fn connect_intervals(repos: &RepositoryRegistry, who: &Person, key: &str, athlete: &str) {
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

/// The Monday at least a week out, so every plan date is in the future on
/// whatever day the suite runs.
fn next_plan_monday() -> NaiveDate {
    let mut day = Utc::now().date_naive() + Duration::days(7);
    while day.weekday() != Weekday::Mon {
        day += Duration::days(1);
    }
    day
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn planned(date: NaiveDate, workout: &str, minutes: u32) -> PlannedDay {
    PlannedDay {
        date: ymd(date),
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

/// The plan week starting next plan Monday, one session named `workout`, in
/// the shape `save_training_plan` takes.
fn week_args(workout: &str) -> Value {
    let monday = next_plan_monday();
    json!({
        "week_start": ymd(monday),
        "focus": workout,
        "days": [{
            "date": ymd(monday + Duration::days(1)),
            "sport": "run",
            "workout": workout,
            "duration_min": 50,
            "intensity": "Z3",
        }],
    })
}

/// The tool's payload, and whether it refused.
async fn call<T: McpTool<dyn ToolRuntime>>(
    res: &Arc<ServerContext>,
    tool: &T,
    who: &Person,
    conversation: Option<String>,
    args: Value,
) -> (Value, bool) {
    let runtime: Arc<dyn ToolRuntime> = Arc::clone(res) as Arc<dyn ToolRuntime>;
    let ctx = ToolContext::new()
        .with_user(who.id.to_string())
        .with_tenant(who.tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = serve_over(
        Transport::WebApp,
        CONVERSATION_ID.scope(conversation, tool.execute(&runtime, &ctx, args)),
    )
    .await;
    (
        response
            .structured_content
            .expect("tool result carries structured content"),
        response.is_error,
    )
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

        let owner = person(&res, "owner@plan-notice.test", "Olive Owner").await;
        let coach = person(&res, COACH_EMAIL, COACH_NAME).await;
        let repos = &res.common.repos;
        connect_intervals(repos, &coach, COACH_KEY, COACH_ATHLETE).await;
        let coach_agent = create_group_agent(repos, coach.id, coach.tenant)
            .await
            .unwrap();
        let group = coached_group(repos, &owner, &coach).await;
        Self {
            res,
            api,
            coach,
            group,
            coach_agent,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// A consenting member of the coach's group named `name`, on the coach's
    /// roster as `athlete`, linked with `calendar_write`, with a live season
    /// derived from data stamped `policy`.
    async fn athlete(&self, name: &str, athlete: &str, policy: TransportPolicy) -> Person {
        let email = format!("{athlete}@plan-notice.test");
        let member = person(&self.res, &email, name).await;
        self.join(&member).await;
        self.api.roster.lock().unwrap().push(
            json!({ "id": athlete, "name": name, "email": email, "icu_permission": "WRITE" }),
        );
        self.link(&member, athlete).await;
        self.season(&member, policy).await;
        member
    }

    async fn join(&self, who: &Person) {
        let now = Utc::now();
        self.repos()
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: self.group.id,
                user_id: who.id,
                tenant_id: who.tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: true,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// A confirmed Intervals.icu link of the coach's for roster athlete
    /// `athlete` as `member`, allowing calendar writes, with the connection
    /// it registers.
    async fn link(&self, member: &Person, athlete: &str) -> DelegatedConnection {
        let repos = self.repos();
        let proposed = repos
            .delegated_connections
            .propose(&DelegatedConnection::propose(
                INTERVALS_ICU.to_owned(),
                self.group.id,
                self.coach.id,
                self.coach.tenant,
                member.id,
                RosterAthlete::new(athlete, None, Some(format!("{athlete}@plan-notice.test"))),
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
                    calendar_write: true,
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

    /// `who`'s live season toward [`RACE`]: two sessions in the week from
    /// next plan Monday, derived from data stamped `policy`.
    async fn season(&self, who: &Person, policy: TransportPolicy) {
        let monday = next_plan_monday();
        let goal = GoalRace {
            name: RACE.to_owned(),
            date: ymd(monday + Duration::days(70)),
            discipline: "trail".to_owned(),
            priority: RacePriority::A,
        };
        let phases = vec![PlanPhase {
            kind: PhaseKind::Base,
            start: ymd(monday),
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
        let start = ymd(monday);
        self.repos()
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &who.tenant.to_string(),
                user_id: &who.id.to_string(),
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

    /// The coach's direct chat: a conversation under their own tenant, bound
    /// to their agent and to no group.
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

    /// `tool` as the coach, from their direct chat in Dravr's web app.
    async fn coach_calls<T: McpTool<dyn ToolRuntime>>(&self, tool: &T, args: Value) -> Value {
        let conversation = self.coach_dm().await;
        let (payload, refused) = call(&self.res, tool, &self.coach, Some(conversation), args).await;
        assert!(!refused, "{payload}");
        payload
    }

    /// The coach's thread bound to `athlete`, opened as the client opens it.
    async fn coach_thread(&self, athlete: &Person) -> String {
        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", athlete.id))
            .header("authorization", &self.coach.auth)
            .json(&json!({ "group_id": self.group.id }))
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        resp.json::<Value>()["id"].as_str().unwrap().to_owned()
    }

    /// The thread's Publish button.
    async fn publish_button(&self, thread: &str, overwrite: bool) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post(&format!(
            "/api/chat/conversations/{thread}/subject-plan/publish"
        ))
        .header("authorization", &self.coach.auth)
        .json(&json!({ "overwrite_athlete_changes": overwrite }))
        .send(ChatRoutes::routes(Arc::clone(&self.res)))
        .await;
        (resp.status_code(), resp.json())
    }

    /// `who`'s coach-plan notices in `tenant`, once `expected` have landed or
    /// the dispatches had their time.
    async fn notices(&self, who: &Person, tenant: TenantId, expected: usize) -> Vec<Notification> {
        let service = self
            .res
            .common
            .notification_service
            .as_ref()
            .expect("the server boots with its notification service");
        let mut rows = Vec::new();
        for _ in 0..40 {
            let (all, _, _) = service
                .list_notifications(who.id, NoticeTenantId(tenant.as_uuid()), 50, 0, None, false)
                .await
                .unwrap();
            rows = all
                .into_iter()
                .filter(|row| row.notification_type == PUBLISHED || row.notification_type == PUSHED)
                .collect();
            if rows.len() >= expected {
                break;
            }
            sleep(StdDuration::from_millis(50)).await;
        }
        rows
    }

    /// The coach was told nothing, in their tenant or the athlete's.
    async fn assert_coach_told_nothing(&self, athlete: &Person) {
        assert!(self
            .notices(&self.coach, self.coach.tenant, 0)
            .await
            .is_empty());
        assert!(self
            .notices(&self.coach, athlete.tenant, 0)
            .await
            .is_empty());
    }
}

/// An active group the owner owns and `coach` coaches.
async fn coached_group(
    repos: &RepositoryRegistry,
    owner: &Person,
    coach: &Person,
) -> CoachingGroup {
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
                name: "Tempo Squad".to_owned(),
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

/// The single notice of kind `wire` in `rows`.
fn only<'a>(rows: &'a [Notification], wire: &str) -> &'a Notification {
    let matching: Vec<&Notification> = rows
        .iter()
        .filter(|row| row.notification_type == wire)
        .collect();
    assert_eq!(matching.len(), 1, "exactly one {wire}: {rows:?}");
    matching[0]
}

/// The published notice: the coach and the race, opening Home, in the
/// athlete's tenant and language.
fn assert_published_notice(row: &Notification, athlete: &Person) {
    assert_eq!(row.tenant_id, NoticeTenantId(athlete.tenant.as_uuid()));
    assert_eq!(row.category, NotificationCategory::Coach);
    assert_eq!(row.title, format!("Plan publié par {COACH_NAME}"));
    assert_eq!(
        row.body,
        format!("{COACH_NAME} a publié ton plan d’entraînement pour {RACE}. Tu le retrouves sur l’Accueil.")
    );
    let data = row.data.as_ref().expect("the notice carries its route");
    assert_eq!(data["screen"], "activities", "{data}");
    assert_eq!(
        data["params"],
        json!({ "coach_name": COACH_NAME, "race_name": RACE }),
        "{data}"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Publish
// ════════════════════════════════════════════════════════════════════════

/// Saving into a draft, discarding it, a publish the athlete's own change
/// refuses and a publish with no draft left tell no one; the publish that
/// puts the draft live tells the athlete, once, and the tool says so.
#[tokio::test]
async fn only_the_publish_that_puts_the_draft_live_tells_the_athlete() {
    let w = World::new().await;
    let alex = w
        .athlete("Alex Athlete", "i201", TransportPolicy::AnyTransport)
        .await;

    // A draft, thrown away.
    let saved = w
        .coach_calls(
            &SaveTrainingPlanTool,
            json!({ "athlete": "Alex", "weeks": [week_args("first draft")] }),
        )
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
    let discarded = w
        .coach_calls(
            &PublishTrainingPlanTool,
            json!({ "athlete": "Alex", "action": "discard" }),
        )
        .await;
    assert_eq!(discarded["status"], "discarded", "{discarded}");

    // A second draft, then the athlete's own agent changes the same week:
    // the publish is refused and writes nothing.
    w.coach_calls(
        &SaveTrainingPlanTool,
        json!({ "athlete": "Alex", "weeks": [week_args("coach draft")] }),
    )
    .await;
    let (changed, refused) = call(
        &w.res,
        &SaveTrainingPlanTool,
        &alex,
        None,
        json!({ "weeks": [week_args("athlete's own change")] }),
    )
    .await;
    assert!(!refused, "{changed}");
    let conflict = w
        .coach_calls(
            &PublishTrainingPlanTool,
            json!({ "athlete": "Alex", "action": "publish" }),
        )
        .await;
    assert_eq!(conflict["status"], "conflict", "{conflict}");
    assert!(conflict.get("athlete_notified").is_none(), "{conflict}");

    // The coach overwrites: the draft is live, and the athlete is told.
    let published = w
        .coach_calls(
            &PublishTrainingPlanTool,
            json!({ "athlete": "Alex", "action": "publish", "overwrite_athlete_changes": true }),
        )
        .await;
    assert_eq!(published["status"], "published", "{published}");
    assert_eq!(published["athlete_notified"], true, "{published}");
    assert!(
        published["message"]
            .as_str()
            .is_some_and(|m| m.contains("was sent a notice")),
        "{published}"
    );
    let rows = w.notices(&alex, alex.tenant, 1).await;
    assert_published_notice(only(&rows, PUBLISHED), &alex);
    assert_eq!(
        only(&rows, PUBLISHED).transport_policy,
        PushTransportPolicy::AnyTransport
    );

    // Nothing left to publish: refused, and still the one notice.
    let dm = w.coach_dm().await;
    let (again, refused) = call(
        &w.res,
        &PublishTrainingPlanTool,
        &w.coach,
        Some(dm),
        json!({ "athlete": "Alex", "action": "publish" }),
    )
    .await;
    assert!(refused, "{again}");
    assert_eq!(again["reason"], "no_draft", "{again}");
    sleep(StdDuration::from_millis(300)).await;
    let rows = w.notices(&alex, alex.tenant, 1).await;
    assert_eq!(rows.len(), 1, "one publish, one notice: {rows:?}");
    // Filed in the athlete's tenant, never the coach's.
    assert!(w.notices(&alex, w.coach.tenant, 0).await.is_empty());
    w.assert_coach_told_nothing(&alex).await;
}

/// The thread's Publish button goes through the same entry point: a 409
/// conflict tells no one, the publish that lands answers
/// `athlete_notified: true` and the athlete holds one notice.
#[tokio::test]
async fn the_publish_button_tells_the_athlete_and_its_conflict_does_not() {
    let w = World::new().await;
    let alex = w
        .athlete("Alex Athlete", "i201", TransportPolicy::AnyTransport)
        .await;
    let thread = w.coach_thread(&alex).await;
    let (saved, refused) = call(
        &w.res,
        &SaveTrainingPlanTool,
        &w.coach,
        Some(thread.clone()),
        json!({ "weeks": [week_args("coach draft")] }),
    )
    .await;
    assert!(!refused, "{saved}");
    assert_eq!(saved["draft"], true, "{saved}");
    call(
        &w.res,
        &SaveTrainingPlanTool,
        &alex,
        None,
        json!({ "weeks": [week_args("athlete's own change")] }),
    )
    .await;

    let (status, body) = w.publish_button(&thread, false).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, body) = w.publish_button(&thread, true).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["athlete_notified"], true, "{body}");
    let rows = w.notices(&alex, alex.tenant, 1).await;
    assert_published_notice(only(&rows, PUBLISHED), &alex);
    sleep(StdDuration::from_millis(300)).await;
    assert_eq!(w.notices(&alex, alex.tenant, 1).await.len(), 1);
    w.assert_coach_told_nothing(&alex).await;
}

/// A season derived from first-party-only data stamps the notice naming its
/// race, so it is never pushed to a device an external caller registered
/// (carnet#769).
#[tokio::test]
async fn publishing_a_first_party_only_season_stamps_its_notice() {
    let w = World::new().await;
    let alex = w
        .athlete("Alex Athlete", "i201", TransportPolicy::FirstPartyOnly)
        .await;
    w.coach_calls(
        &SaveTrainingPlanTool,
        json!({ "athlete": "Alex", "weeks": [week_args("coach draft")] }),
    )
    .await;
    // The button runs in no turn whose derivation could stamp the notice:
    // the stamp is the published season's own.
    let thread = w.coach_thread(&alex).await;
    let (status, body) = w.publish_button(&thread, false).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["athlete_notified"], true, "{body}");

    let rows = w.notices(&alex, alex.tenant, 1).await;
    let notice = only(&rows, PUBLISHED);
    assert_published_notice(notice, &alex);
    assert_eq!(notice.transport_policy, PushTransportPolicy::FirstPartyOnly);
}

// ════════════════════════════════════════════════════════════════════════
// Push
// ════════════════════════════════════════════════════════════════════════

/// The coach's push that put workouts on the athlete's calendar tells them,
/// naming the coach and the calendar and opening that connection; the next
/// push finds every entry current, writes nothing and tells no one.
#[tokio::test]
async fn a_coachs_push_that_wrote_workouts_tells_the_athlete_once() {
    let w = World::new().await;
    let alex = w
        .athlete("Alex Athlete", "i201", TransportPolicy::AnyTransport)
        .await;

    let report = w
        .coach_calls(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    // Two sessions and the week's note.
    assert_eq!(report["created"], 3, "{report}");

    let rows = w.notices(&alex, alex.tenant, 1).await;
    let notice = only(&rows, PUSHED);
    assert_eq!(notice.tenant_id, NoticeTenantId(alex.tenant.as_uuid()));
    assert_eq!(notice.category, NotificationCategory::Coach);
    assert_eq!(
        notice.title,
        "Séances ajoutées à ton calendrier Intervals.icu"
    );
    assert_eq!(
        notice.body,
        format!("{COACH_NAME} a ajouté les séances de ton plan à ton calendrier Intervals.icu.")
    );
    let data = notice.data.as_ref().expect("the notice carries its route");
    assert_eq!(data["screen"], "connections", "{data}");
    assert_eq!(data["provider"], INTERVALS_ICU, "{data}");
    assert_eq!(
        data["params"],
        json!({ "coach_name": COACH_NAME, "provider_name": "Intervals.icu" }),
        "{data}"
    );

    // Everything is already on the calendar: nothing written, nothing told.
    let again = w
        .coach_calls(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert_eq!(again["created"], 0, "{again}");
    assert_eq!(again["updated"], 0, "{again}");
    assert_eq!(again["unchanged"], 3, "{again}");
    sleep(StdDuration::from_millis(300)).await;
    assert_eq!(w.notices(&alex, alex.tenant, 1).await.len(), 1);
    w.assert_coach_told_nothing(&alex).await;
}

/// A push Intervals.icu refused wrote nothing and tells no one, nor does a
/// self-coached athlete's push of their own plan; the coach's push once the
/// calendar takes writes again is the one notice.
#[tokio::test]
async fn a_refused_push_and_an_athletes_own_push_tell_no_one() {
    let w = World::new().await;
    let alex = w
        .athlete("Alex Athlete", "i201", TransportPolicy::AnyTransport)
        .await;

    // A self-coached athlete with their own key pushes their own plan.
    let sam = person(&w.res, "sam@plan-notice.test", "Sam Self").await;
    connect_intervals(w.repos(), &sam, SAM_KEY, "i301").await;
    w.season(&sam, TransportPolicy::AnyTransport).await;
    let (own, refused) = call(&w.res, &PushTrainingPlanTool, &sam, None, json!({})).await;
    assert!(!refused, "{own}");
    assert_eq!(own["created"], 3, "{own}");

    // The coach's push is refused entry by entry.
    w.api.refuse_writes("i201", true);
    let refused_push = w
        .coach_calls(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert_eq!(refused_push["created"], 0, "{refused_push}");
    assert_eq!(
        refused_push["failed"].as_array().map(Vec::len),
        Some(3),
        "{refused_push}"
    );

    w.api.refuse_writes("i201", false);
    let landed = w
        .coach_calls(&PushTrainingPlanTool, json!({ "athlete": "Alex" }))
        .await;
    assert_eq!(landed["created"], 3, "{landed}");
    let rows = w.notices(&alex, alex.tenant, 1).await;
    only(&rows, PUSHED);
    sleep(StdDuration::from_millis(300)).await;
    assert_eq!(
        w.notices(&alex, alex.tenant, 1).await.len(),
        1,
        "the refused push told no one"
    );
    assert!(
        w.notices(&sam, sam.tenant, 0).await.is_empty(),
        "an athlete's own push tells no one"
    );
    w.assert_coach_told_nothing(&alex).await;
}
