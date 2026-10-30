// ABOUTME: A human coach's draft of a coached athlete's plan, end to end through the plan tools (carnet#1069)
// ABOUTME: Every athlete surface shows only the live plan until publish; conflicts, discard, consent and two coaches are pinned

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A group's human coach builds an athlete's plan in a draft the athlete
//! never sees, then publishes it in one step. These tests drive the coach
//! through the tools in their thread about the athlete (carnet#1070) and read
//! the athlete back through every surface they have — Home's plan, the
//! calendar strip, `/plan`, the plan card, their agent's prompt and the
//! starters' state — so a draft that leaked anywhere fails here, not in an
//! athlete's hands.
//!
//! A draft save also answers with the safety findings a live save reports —
//! the opening week's ramp and crowded hard days — measured on the draft
//! view, while emitting none of the live save's telemetry.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Datelike, Days, NaiveDate, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_logs, CapturedEvents};
use pierre_chat_pipeline::stages::memory::{inject_training_plan, PlanPromptSources};
use pierre_commands::plan::PlanShowHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_contremaitre::TrainingCatalogueRegistry;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_services::athlete_state::AthleteState;
use pierre_services::plan_card::load_plan_card;
use pierre_test_support::delegation::create_group_agent;
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plan_drafts::PublishTrainingPlanTool;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

const SUBJECT_NAME: &str = "Mira Subject";
const ATHLETE_RACE: &str = "Athlete Season Race";
const COACH_RACE: &str = "Coach Draft Race";
const COACH_WORKOUT: &str = "coach-drafted tempo 3x10min";
const ATHLETE_WORKOUT: &str = "athlete easy run";

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    token: String,
}

/// A group its owner owns and `coach` coaches, with the subject a
/// consenting member, and the coach's thread about the subject open.
struct Fixture {
    res: Arc<ServerContext>,
    owner: Person,
    coach: Person,
    subject: Person,
    group: CoachingGroup,
    thread: String,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@plan-draft.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person { id, tenant, token }
}

/// A group `owner` owns and `coach` is attached to, `members` consenting.
async fn coached_group(
    repos: &RepositoryRegistry,
    owner: &Person,
    coach: &Person,
    members: &[&Person],
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
    // Redeeming a coach invite attaches the coach through this write.
    assert!(repos
        .groups
        .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
        .await
        .unwrap());
    for member in members {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: member.id,
                tenant_id: member.tenant.to_string(),
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
    group
}

/// The Monday `weeks` weeks after this one's, in the athlete's today (UTC:
/// no zone is set on them).
fn monday(weeks: u64) -> NaiveDate {
    let today = Utc::now().date_naive();
    let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    this + Days::new(7 * weeks)
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// One week of seven sessions all called `workout`, so whatever day today is,
/// the athlete's compact `/plan` shows it.
fn week(start: NaiveDate, focus: &str, workout: &str) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Days::new(offset)),
                "sport": "run",
                "workout": workout,
                "duration_min": 40,
                "intensity": "Z2",
            })
        })
        .collect();
    json!({ "week_start": ymd(start), "focus": focus, "days": days })
}

fn outline(race: &str) -> Value {
    json!({
        "goal_race": { "name": race, "date": "2027-05-16", "discipline": "trail", "priority": "A" },
        "strategy": format!("build toward {race}"),
    })
}

fn tool_context(who: &Person) -> ToolContext {
    ToolContext::new()
        .with_user(who.id.to_string())
        .with_tenant(who.tenant.to_string())
        .with_auth_method("jwt_bearer")
}

/// Call `tool` as `who`, inside `conversation` when there is one.
async fn call<T: McpTool<dyn ToolRuntime>>(
    res: &Arc<ServerContext>,
    who: &Person,
    tool: &T,
    conversation: Option<&str>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(res);
    let ctx = tool_context(who);
    CONVERSATION_ID
        .scope(conversation.map(ToOwned::to_owned), async {
            tool.execute(&runtime, &ctx, args)
                .await
                .structured_content
                .expect("tool result carries structured content")
        })
        .await
}

impl Fixture {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", "Casey Coach").await;
        let subject = person(&res, "subject", SUBJECT_NAME).await;
        let group = coached_group(&res.common.repos, &owner, &coach, &[&subject]).await;
        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", subject.id))
            .header("authorization", &format!("Bearer {}", coach.token))
            .json(&json!({ "group_id": group.id }))
            .send(ChatRoutes::routes(Arc::clone(&res)))
            .await;
        assert_eq!(resp.status_code(), StatusCode::OK);
        let thread = resp.json::<Value>()["id"].as_str().unwrap().to_owned();
        Self {
            res,
            owner,
            coach,
            subject,
            group,
            thread,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// The athlete's own agent saves, from no conversation: their own season.
    async fn athlete_saves(&self, args: Value) -> Value {
        let saved = call(&self.res, &self.subject, &SaveTrainingPlanTool, None, args).await;
        assert!(saved.get("error").is_none(), "{saved}");
        saved
    }

    /// The athlete lays their season with this week and next.
    async fn athlete_season(&self) {
        self.athlete_saves(json!({
            "outline": outline(ATHLETE_RACE),
            "weeks": [
                week(monday(0), "athlete week zero", ATHLETE_WORKOUT),
                week(monday(1), "athlete week one", ATHLETE_WORKOUT),
            ],
        }))
        .await;
    }

    /// The coach saves in their thread about the athlete, naming no one.
    async fn coach_saves(&self, args: Value) -> Value {
        let saved = call(
            &self.res,
            &self.coach,
            &SaveTrainingPlanTool,
            Some(&self.thread),
            args,
        )
        .await;
        assert!(saved.get("error").is_none(), "{saved}");
        saved
    }

    async fn coach_reads(&self) -> Value {
        call(
            &self.res,
            &self.coach,
            &GetTrainingPlanTool,
            Some(&self.thread),
            json!({}),
        )
        .await
    }

    async fn coach_publishes(&self, args: Value) -> Value {
        call(
            &self.res,
            &self.coach,
            &PublishTrainingPlanTool,
            Some(&self.thread),
            args,
        )
        .await
    }

    /// `GET /api/me/training-plan` as the athlete.
    async fn home(&self) -> Value {
        self.athlete_get("/api/me/training-plan").await
    }

    /// `GET /api/me/calendar` over this week and next, as the athlete.
    async fn calendar(&self) -> Value {
        self.athlete_get(&format!(
            "/api/me/calendar?from={}&to={}",
            ymd(monday(0)),
            ymd(monday(1) + Days::new(6))
        ))
        .await
    }

    async fn athlete_get(&self, uri: &str) -> Value {
        let response = athlete_home_routes()
            .with_state(Arc::clone(&self.res))
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("authorization", format!("Bearer {}", self.subject.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{uri}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The athlete's `/plan`, from a DM of their own.
    async fn plan_command(&self) -> String {
        let conversation = self
            .repos()
            .chat
            .create_conversation(
                &self.subject.id.to_string(),
                self.subject.tenant,
                "dm",
                "gemini-2.0-flash",
                None,
                None,
            )
            .await
            .unwrap();
        let ctx = PlatformCommandContext {
            user_id: self.subject.id,
            tenant_id: self.subject.tenant,
            channel_type: "telegram".to_owned(),
            args: vec![],
            raw_text: "/plan".to_owned(),
            ctx: Arc::<ServerContext>::clone(&self.res),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: Some(conversation.id),
            conversation_tenant_id: self.subject.tenant,
            sender_id: None,
            rotation: ConversationRotation::default(),
            tool_runtime: Arc::<ServerContext>::clone(&self.res),
        };
        PlanShowHandler.execute(&ctx).await.unwrap().text
    }

    /// Every surface the athlete reads their plan on.
    async fn athlete_surfaces(&self) -> Surfaces {
        let repos = self.repos();
        let today = Utc::now().date_naive();
        let card = load_plan_card(
            repos,
            self.subject.tenant,
            self.subject.id,
            today,
            &self.res.mcp.messaging_strings_registry,
            "en",
        )
        .await;
        let prompt = inject_training_plan(
            PlanPromptSources {
                repos,
                catalogue: &TrainingCatalogueRegistry::new(),
            },
            &self.subject.tenant.to_string(),
            &self.subject.id.to_string(),
            None,
            today,
            false,
            "BASE PROMPT".to_owned(),
        )
        .await;
        let (state, _) =
            AthleteState::read(repos, self.subject.tenant, self.subject.id, Utc::now())
                .await
                .unwrap();
        Surfaces {
            home: self.home().await,
            calendar: self.calendar().await,
            plan_command: self.plan_command().await,
            card: serde_json::to_value(&card).unwrap(),
            prompt,
            season_set: state.season_set,
            plan_active: state.plan_active,
        }
    }
}

/// What the athlete sees of their plan, surface by surface.
struct Surfaces {
    home: Value,
    calendar: Value,
    plan_command: String,
    card: Value,
    prompt: String,
    season_set: bool,
    plan_active: bool,
}

impl Surfaces {
    /// No surface mentions `text`.
    fn assert_none_shows(&self, text: &str) {
        assert!(!self.home.to_string().contains(text), "Home: {}", self.home);
        assert!(
            !self.calendar.to_string().contains(text),
            "calendar: {}",
            self.calendar
        );
        assert!(
            !self.plan_command.contains(text),
            "/plan: {}",
            self.plan_command
        );
        assert!(!self.card.to_string().contains(text), "card: {}", self.card);
        assert!(!self.prompt.contains(text), "prompt: {}", self.prompt);
    }

    /// Every surface shows `text`.
    fn assert_all_show(&self, text: &str) {
        assert!(self.home.to_string().contains(text), "Home: {}", self.home);
        assert!(
            self.calendar.to_string().contains(text),
            "calendar: {}",
            self.calendar
        );
        assert!(
            self.plan_command.contains(text),
            "/plan: {}",
            self.plan_command
        );
        assert!(self.card.to_string().contains(text), "card: {}", self.card);
        assert!(self.prompt.contains(text), "prompt: {}", self.prompt);
    }
}

/// The workouts of the live week starting `start`, from Home's card.
fn home_week(home: &Value, start: NaiveDate) -> Vec<String> {
    home["plan"]["weeks"]
        .as_array()
        .unwrap_or_else(|| panic!("Home's plan has weeks: {home}"))
        .iter()
        .find(|w| w["week_start"] == ymd(start))
        .unwrap_or_else(|| panic!("Home shows the week of {}: {home}", ymd(start)))["days"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["workout"].as_str().unwrap().to_owned())
        .collect()
}

// ════════════════════════════════════════════════════════════════════════
// A draft reaches no surface of the athlete's
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_coachs_draft_of_a_first_season_reaches_no_athlete_surface() {
    let fx = Fixture::new().await;

    let saved = fx
        .coach_saves(json!({
            "outline": outline(COACH_RACE),
            "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)],
        }))
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
    assert_eq!(saved["athlete"], SUBJECT_NAME, "{saved}");
    assert!(
        saved["message"]
            .as_str()
            .unwrap()
            .contains("does not see it until you publish it with publish_training_plan"),
        "{saved}"
    );

    let surfaces = fx.athlete_surfaces().await;
    assert!(surfaces.home["plan"].is_null(), "{}", surfaces.home);
    assert!(
        surfaces.calendar["plan_weeks"].is_null(),
        "{}",
        surfaces.calendar
    );
    assert!(surfaces.card.is_null(), "{}", surfaces.card);
    assert_eq!(surfaces.prompt, "BASE PROMPT");
    assert!(!surfaces.season_set && !surfaces.plan_active);
    surfaces.assert_none_shows(COACH_WORKOUT);
    surfaces.assert_none_shows(COACH_RACE);

    // The coach reads their draft: no live plan, the draft beside it.
    let read = fx.coach_reads().await;
    assert!(read["plan"].is_null(), "{read}");
    assert_eq!(read["draft"]["plan"]["status"], "draft", "{read}");
    assert_eq!(read["draft"]["plan"]["goal_race"]["name"], COACH_RACE);
    assert_eq!(read["draft"]["state"]["season_relaid"], false, "{read}");
    assert!(read["message"]
        .as_str()
        .unwrap()
        .contains("your draft below is not published"));
}

#[tokio::test]
async fn a_coachs_draft_over_a_season_leaves_every_athlete_surface_on_the_live_plan() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;

    fx.coach_saves(json!({
        "outline": outline(COACH_RACE),
        "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)],
    }))
    .await;

    let surfaces = fx.athlete_surfaces().await;
    surfaces.assert_none_shows(COACH_WORKOUT);
    surfaces.assert_none_shows(COACH_RACE);
    surfaces.assert_all_show(ATHLETE_WORKOUT);
    assert_eq!(surfaces.home["plan"]["goal_race"]["name"], ATHLETE_RACE);
    assert!(surfaces.season_set && surfaces.plan_active);

    // The coach reads the live plan as the athlete has it, and their draft:
    // the re-laid outline over the live weeks, with the drafted one in place.
    let read = fx.coach_reads().await;
    assert_eq!(read["plan"]["goal_race"]["name"], ATHLETE_RACE, "{read}");
    assert!(!read["weeks"].to_string().contains(COACH_WORKOUT), "{read}");
    let draft = &read["draft"];
    assert_eq!(draft["plan"]["goal_race"]["name"], COACH_RACE, "{read}");
    let weeks = draft["weeks"].as_array().unwrap();
    assert_eq!(weeks.len(), 2, "{draft}");
    assert_eq!(weeks[0]["week_start"], ymd(monday(0)));
    assert!(weeks[0].to_string().contains(COACH_WORKOUT));
    assert_eq!(weeks[1]["focus"], "athlete week one");
    assert_eq!(draft["drafted_weeks"], json!([ymd(monday(0))]));
    assert_eq!(draft["state"]["conflicts"], json!([]));
    assert_eq!(draft["state"]["season_relaid"], false);
}

// ════════════════════════════════════════════════════════════════════════
// Publish
// ════════════════════════════════════════════════════════════════════════

/// In the coach's thread about the athlete, publish names no one; the weeks
/// the coach drafted go live and the ones they did not keep the athlete's
/// own, newer, version.
#[tokio::test]
async fn publishing_puts_the_draft_live_and_untouched_weeks_keep_the_athletes_saves() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    fx.coach_saves(json!({ "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)] }))
        .await;

    // The athlete's own agent changes the week the coach did not touch: their
    // save is live at once, draft or no draft.
    let saved = fx
        .athlete_saves(json!({ "weeks": [week(monday(1), "athlete moved", "athlete moved run")] }))
        .await;
    assert!(saved.get("draft").is_none(), "{saved}");
    assert_eq!(
        home_week(&fx.home().await, monday(1))[0],
        "athlete moved run",
        "the athlete's own save is live before any publish"
    );

    let published = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(published["status"], "published", "{published}");
    assert_eq!(published["athlete"], SUBJECT_NAME);
    assert_eq!(published["weeks_published"], json!([ymd(monday(0))]));
    // The athlete is told whenever the server sends notices at all
    // (`coach_plan_notice_test` reads the notice back).
    assert_eq!(
        published["athlete_notified"],
        cfg!(feature = "client-notifications"),
        "{published}"
    );
    assert!(published["superseded_plan_id"].is_null(), "{published}");

    let home = fx.home().await;
    assert_eq!(home["plan"]["goal_race"]["name"], ATHLETE_RACE, "{home}");
    assert_eq!(home_week(&home, monday(0))[0], COACH_WORKOUT);
    assert_eq!(home_week(&home, monday(1))[0], "athlete moved run");
    fx.athlete_surfaces().await.assert_all_show(COACH_WORKOUT);

    // The draft is spent: the coach reads none, and a second publish has
    // nothing to publish.
    let read = fx.coach_reads().await;
    assert!(read.get("draft").is_none(), "{read}");
    let again = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(again["reason"], "no_draft", "{again}");
}

#[tokio::test]
async fn publishing_a_relaid_outline_relays_the_athletes_season() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    let live = fx
        .repos()
        .training_plans
        .get_active_plan(&fx.subject.tenant.to_string(), &fx.subject.id.to_string())
        .await
        .unwrap()
        .unwrap();
    fx.coach_saves(json!({ "outline": outline(COACH_RACE) }))
        .await;

    let published = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(published["status"], "published", "{published}");
    assert_eq!(published["superseded_plan_id"], live.id.as_str());
    let home = fx.home().await;
    assert_eq!(home["plan"]["goal_race"]["name"], COACH_RACE, "{home}");
    // The season's weeks are carried onto the re-laid outline.
    assert_eq!(home_week(&home, monday(0))[0], ATHLETE_WORKOUT);
}

#[tokio::test]
async fn a_week_both_changed_is_refused_then_published_with_overwrite() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    fx.coach_saves(json!({ "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)] }))
        .await;
    fx.athlete_saves(json!({ "weeks": [week(monday(0), "athlete again", "athlete again run")] }))
        .await;

    // The coach's read already says what publishing would overwrite.
    let read = fx.coach_reads().await;
    assert_eq!(
        read["draft"]["state"]["conflicts"],
        json!([{ "week_start": ymd(monday(0)), "reason": "athlete_changed_week" }]),
        "{read}"
    );

    let refused = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(refused["status"], "conflict", "{refused}");
    assert_eq!(
        refused["conflict"]["conflicts"],
        json!([{ "week_start": ymd(monday(0)), "reason": "athlete_changed_week" }])
    );
    assert_eq!(refused["conflict"]["season_relaid"], false);
    assert_eq!(refused["draft_id"], read["draft"]["state"]["draft_id"]);
    assert_eq!(
        home_week(&fx.home().await, monday(0))[0],
        "athlete again run",
        "a refused publish writes nothing"
    );

    let published = fx
        .coach_publishes(json!({ "action": "publish", "overwrite_athlete_changes": true }))
        .await;
    assert_eq!(published["status"], "published", "{published}");
    assert_eq!(home_week(&fx.home().await, monday(0))[0], COACH_WORKOUT);
}

#[tokio::test]
async fn a_season_relaid_under_the_draft_is_refused() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    fx.coach_saves(json!({ "weeks": [week(monday(1), "coach week one", COACH_WORKOUT)] }))
        .await;
    // The athlete's own agent re-lays their season.
    fx.athlete_saves(json!({ "outline": outline("Athlete Relaid Race") }))
        .await;

    let refused = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(refused["status"], "conflict", "{refused}");
    assert_eq!(refused["conflict"]["season_relaid"], true, "{refused}");
    fx.athlete_surfaces().await.assert_none_shows(COACH_WORKOUT);

    let published = fx
        .coach_publishes(json!({ "action": "publish", "overwrite_athlete_changes": true }))
        .await;
    assert_eq!(published["status"], "published", "{published}");
    let home = fx.home().await;
    assert_eq!(home["plan"]["goal_race"]["name"], "Athlete Relaid Race");
    assert_eq!(home_week(&home, monday(1))[0], COACH_WORKOUT);
}

// ════════════════════════════════════════════════════════════════════════
// The safety report on the draft view, and no telemetry for a draft
// ════════════════════════════════════════════════════════════════════════

/// Six rides inside the six-week snapshot window, one a week, each `seconds`
/// long: the athlete's recent load is exactly that much a week, so a planned
/// week's ramp verdict is arithmetic rather than a guess.
async fn seed_weekly_load(fx: &Fixture, seconds: u64) {
    let rides: Vec<Activity> = (0..6)
        .map(|i| {
            ActivityBuilder::new(
                format!("baseline-{i}"),
                format!("baseline ride {i}"),
                SportType::Ride,
                Utc::now() - chrono::Duration::days(2 + i * 7),
                seconds,
                "strava",
            )
            .build()
        })
        .collect();
    fx.repos()
        .activity_cache
        .upsert_activities(fx.subject.id, &fx.subject.tenant, "strava", &rides)
        .await
        .unwrap();
}

/// An outline on the catalogue's polarized flavour — whose minimum puts 48 h
/// between hard sessions — with one build phase from this week, so the
/// compliance walk has a flavour to measure spacing against.
fn flavoured_outline(race: &str) -> Value {
    json!({
        "goal_race": { "name": race, "date": "2027-05-16", "discipline": "trail", "priority": "A" },
        "strategy": format!("polarised build toward {race}"),
        "flavour": { "id": "polarized-classic", "selected_by": "coach", "override_reason": "house style" },
        "phases": [
            { "kind": "build", "start": ymd(monday(0)), "weeks": 6, "intent": "two hard days, the rest easy" }
        ],
    })
}

/// A quality session built from the catalogue's threshold template.
fn threshold_day(date: NaiveDate) -> Value {
    json!({
        "date": ymd(date),
        "sport": "run",
        "workout": "4 x 8 min at threshold",
        "duration_min": 75,
        "intensity": "threshold",
        "template_slug": "threshold_4x8",
        "template_params": { "reps": 4, "work_seconds": 480, "rest_seconds": 120 },
    })
}

fn easy_day(date: NaiveDate, minutes: u32) -> Value {
    json!({ "date": ymd(date), "sport": "run", "workout": "easy", "duration_min": minutes, "intensity": "Z2" })
}

/// A week of the flavoured outline's build phase holding `days`.
fn phase_week(start: NaiveDate, focus: &str, days: &[Value]) -> Value {
    json!({ "week_start": ymd(start), "focus": focus, "phase_index": 0, "days": days })
}

/// Seven hours whose two thresholds sit on back-to-back days: 24 h apart
/// against the flavour's 48 h minimum, and far above a one-hour week.
fn crowded_heavy_week(start: NaiveDate, focus: &str) -> Value {
    phase_week(
        start,
        focus,
        &[
            threshold_day(start),
            threshold_day(start + Days::new(1)),
            easy_day(start + Days::new(2), 90),
            easy_day(start + Days::new(3), 90),
            easy_day(start + Days::new(4), 90),
        ],
    )
}

/// Every plan-write telemetry line a captured run emitted: each
/// `training_plan.*` notify event, and the readiness rail's verdict line.
fn plan_telemetry(events: &CapturedEvents) -> Vec<String> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|line| match line.fields.get("event") {
            Some(name) if name.starts_with("training_plan.") => Some(name.clone()),
            _ => (line.event == "week readiness assessed").then(|| line.event.clone()),
        })
        .collect()
}

/// The spacing word each `training_plan.week_assessed` line reported, by week.
fn assessed_spacing(events: &CapturedEvents) -> Vec<(String, String)> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|line| {
            line.fields.get("event").map(String::as_str) == Some("training_plan.week_assessed")
        })
        .map(|line| {
            (
                line.field("week_start").to_owned(),
                line.field("spacing").to_owned(),
            )
        })
        .collect()
}

/// The coach learns of a ramp spike while the draft is still theirs: the
/// opening week of the outline they drafted is measured against the
/// athlete's own recent load, as the athlete's own save is. A weeks-only
/// save is not an opening week, on the draft path as on the live one.
#[tokio::test]
async fn a_draft_whose_opening_week_far_exceeds_the_athletes_load_reports_the_ramp() {
    let fx = Fixture::new().await;
    seed_weekly_load(&fx, 3_600).await;

    // Seven 40-minute sessions: 4.7 h against a one-hour week.
    let saved = fx
        .coach_saves(json!({
            "outline": outline(COACH_RACE),
            "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)],
        }))
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
    let ramp = &saved["safety"]["ramp"];
    assert_eq!(ramp["outcome"], "exceeded", "{saved}");
    assert_eq!(
        ramp["baseline_hours"], 1.0,
        "the athlete's load, not the coach's: {saved}"
    );
    let planned = ramp["planned_hours"].as_f64().unwrap();
    assert!(
        (planned - 280.0 / 60.0).abs() < 1e-9,
        "the drafted opening week is the one measured: {saved}"
    );
    assert!(
        saved["safety"].get("crowded_weeks").is_none(),
        "no flavour, no spacing finding: {saved}"
    );

    let adjusted = fx
        .coach_saves(json!({ "weeks": [week(monday(1), "coach week one", COACH_WORKOUT)] }))
        .await;
    assert_eq!(adjusted["draft"], true, "{adjusted}");
    assert!(
        adjusted.get("safety").is_none(),
        "a weeks-only draft save grades no opening week: {adjusted}"
    );
}

/// The compliance walk runs on the draft view: the athlete's live weeks with
/// the drafted one in place. Within the drafted week the two thresholds are
/// 48 h apart — the minimum — so only the athlete's live Sunday threshold the
/// day before makes it crowd; a walk over the drafted week alone would find
/// nothing.
#[tokio::test]
async fn a_drafted_week_crowding_the_live_weeks_last_hard_day_is_reported() {
    let fx = Fixture::new().await;
    let sunday = monday(0) + Days::new(6);
    fx.athlete_saves(json!({
        "outline": flavoured_outline(ATHLETE_RACE),
        "weeks": [phase_week(
            monday(0),
            "athlete week zero",
            &[easy_day(monday(0), 60), easy_day(monday(0) + Days::new(2), 60), threshold_day(sunday)],
        )],
    }))
    .await;

    let saved = fx
        .coach_saves(json!({
            "weeks": [phase_week(
                monday(1),
                "coach week one",
                &[
                    threshold_day(monday(1)),
                    easy_day(monday(1) + Days::new(1), 60),
                    threshold_day(monday(1) + Days::new(2)),
                ],
            )],
        }))
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
    let crowded = saved["safety"]["crowded_weeks"]
        .as_array()
        .unwrap_or_else(|| panic!("the draft save reports its crowded week: {saved}"));
    assert_eq!(crowded.len(), 1, "{saved}");
    assert_eq!(crowded[0]["week_start"], ymd(monday(1)));
    assert_eq!(crowded[0]["min_hours"], 48, "{saved}");
    assert!(
        crowded[0]["short"]
            .as_array()
            .is_some_and(|gaps| !gaps.is_empty()),
        "the short gap travels with the finding: {saved}"
    );
    assert!(
        saved["safety"].get("ramp").is_none(),
        "a weeks-only save grades no opening week: {saved}"
    );
}

/// The control: a draft both checks measure and find nothing in — an opening
/// week matching the athlete's load, hard days 48 h apart on a flavour —
/// carries no `safety` at all. Without it, the reports above pass just as well
/// when every draft save reports something.
#[tokio::test]
async fn a_clean_draft_reports_no_safety() {
    let fx = Fixture::new().await;
    // 4.5 h a week, the drafted week's own hours.
    seed_weekly_load(&fx, 16_200).await;

    let saved = fx
        .coach_saves(json!({
            "outline": flavoured_outline(COACH_RACE),
            "weeks": [phase_week(
                monday(0),
                "coach week zero",
                &[
                    threshold_day(monday(0)),
                    easy_day(monday(0) + Days::new(1), 60),
                    threshold_day(monday(0) + Days::new(2)),
                    easy_day(monday(0) + Days::new(3), 60),
                ],
            )],
        }))
        .await;
    assert_eq!(saved["draft"], true, "{saved}");
    assert!(saved.get("safety").is_none(), "{saved}");
}

/// The same payload, saved by the athlete's own agent and then drafted by
/// the coach, finds the same two things. Only the live write reports itself:
/// the draft save emits no plan telemetry, and publishing it runs the rails.
#[tokio::test]
async fn a_draft_save_emits_no_plan_telemetry_while_the_live_save_and_publish_do() {
    let fx = Fixture::new().await;
    seed_weekly_load(&fx, 3_600).await;

    let (events, guard) = capture_logs();
    let live = fx
        .athlete_saves(json!({
            "outline": flavoured_outline(ATHLETE_RACE),
            "weeks": [crowded_heavy_week(monday(0), "athlete heavy week")],
        }))
        .await;
    drop(guard);
    assert!(live.get("draft").is_none(), "{live}");
    assert_eq!(live["safety"]["ramp"]["outcome"], "exceeded", "{live}");
    assert_eq!(
        live["safety"]["crowded_weeks"][0]["week_start"],
        ymd(monday(0)),
        "{live}"
    );
    let emitted = plan_telemetry(&events);
    for event in ["training_plan.saved", "training_plan.ramp_warning"] {
        assert!(
            emitted.iter().any(|name| name == event),
            "the athlete's own save emits {event}: {emitted:?}"
        );
    }
    assert_eq!(
        assessed_spacing(&events),
        vec![(ymd(monday(0)), "off".to_owned())],
        "the compliance rail reports the live week"
    );

    let (events, guard) = capture_logs();
    let drafted = fx
        .coach_saves(json!({
            "outline": flavoured_outline(COACH_RACE),
            "weeks": [crowded_heavy_week(monday(0), "coach heavy week")],
        }))
        .await;
    drop(guard);
    assert_eq!(drafted["draft"], true, "{drafted}");
    assert_eq!(
        drafted["safety"], live["safety"],
        "one assessment: the draft finds what the live save found"
    );
    assert_eq!(
        plan_telemetry(&events),
        Vec::<String>::new(),
        "a draft save reports nothing as a write to the athlete's plan"
    );

    let (events, guard) = capture_logs();
    let published = fx.coach_publishes(json!({ "action": "publish" })).await;
    drop(guard);
    assert_eq!(published["status"], "published", "{published}");
    assert_eq!(published["safety"], live["safety"], "{published}");
    let emitted = plan_telemetry(&events);
    for event in [
        "training_plan.published",
        "training_plan.saved",
        "training_plan.ramp_warning",
    ] {
        assert!(
            emitted.iter().any(|name| name == event),
            "publishing runs the rails and emits {event}: {emitted:?}"
        );
    }
    assert_eq!(
        assessed_spacing(&events),
        vec![(ymd(monday(0)), "off".to_owned())],
        "the compliance rail reports the published week"
    );
}

// ════════════════════════════════════════════════════════════════════════
// Discard, consent, two coaches, the athlete's own plan
// ════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a_discarded_draft_is_gone_and_the_athletes_plan_is_untouched() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    fx.coach_saves(json!({ "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)] }))
        .await;

    let discarded = fx.coach_publishes(json!({ "action": "discard" })).await;
    assert_eq!(discarded["status"], "discarded", "{discarded}");
    assert!(fx.coach_reads().await.get("draft").is_none());
    fx.athlete_surfaces().await.assert_none_shows(COACH_WORKOUT);
    let again = fx.coach_publishes(json!({ "action": "discard" })).await;
    assert_eq!(again["reason"], "no_draft", "{again}");
    let publish = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(publish["reason"], "no_draft", "{publish}");
}

#[tokio::test]
async fn a_consent_revoked_between_draft_and_publish_refuses_the_publish() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    fx.coach_saves(json!({ "weeks": [week(monday(0), "coach week zero", COACH_WORKOUT)] }))
        .await;

    assert!(fx
        .repos()
        .groups
        .update_coach_sharing_consent(&fx.group.id.to_string(), fx.subject.id, false)
        .await
        .unwrap());
    let refused = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("has stopped sharing their data with you"),
        "{refused}"
    );
    fx.athlete_surfaces().await.assert_none_shows(COACH_WORKOUT);
}

#[tokio::test]
async fn two_coaches_drafts_are_their_own_until_one_publishes() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;
    let second = person(&fx.res, "coach-two", "Dana Second").await;
    coached_group(fx.repos(), &fx.owner, &second, &[&fx.subject]).await;
    let second_dm = fx
        .repos()
        .chat
        .create_conversation(
            &second.id.to_string(),
            second.tenant,
            "second coach dm",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap()
        .id;

    fx.coach_saves(json!({ "weeks": [week(monday(0), "first coach", "first coach run")] }))
        .await;
    let saved = call(
        &fx.res,
        &second,
        &SaveTrainingPlanTool,
        Some(&second_dm),
        json!({
            "athlete": "mira",
            "weeks": [week(monday(0), "second coach", "second coach run")],
        }),
    )
    .await;
    assert_eq!(saved["draft"], true, "{saved}");

    // Each coach reads their own draft.
    let first_read = fx.coach_reads().await;
    assert!(first_read["draft"].to_string().contains("first coach run"));
    assert!(!first_read["draft"].to_string().contains("second coach run"));
    let second_read = call(
        &fx.res,
        &second,
        &GetTrainingPlanTool,
        Some(&second_dm),
        json!({ "athlete": "mira" }),
    )
    .await;
    assert!(second_read["draft"]
        .to_string()
        .contains("second coach run"));
    assert_ne!(
        first_read["draft"]["state"]["draft_id"],
        second_read["draft"]["state"]["draft_id"]
    );

    // The first publishes; the second drafted over the week that just
    // changed, and is told so.
    let published = fx.coach_publishes(json!({ "action": "publish" })).await;
    assert_eq!(published["status"], "published", "{published}");
    let refused = call(
        &fx.res,
        &second,
        &PublishTrainingPlanTool,
        Some(&second_dm),
        json!({ "athlete": "mira", "action": "publish" }),
    )
    .await;
    assert_eq!(refused["status"], "conflict", "{refused}");
    assert_eq!(
        refused["conflict"]["conflicts"][0]["week_start"],
        ymd(monday(0))
    );
    assert_eq!(home_week(&fx.home().await, monday(0))[0], "first coach run");
}

#[tokio::test]
async fn the_athletes_own_plan_has_no_draft_to_publish() {
    let fx = Fixture::new().await;
    fx.athlete_season().await;

    let refused = call(
        &fx.res,
        &fx.subject,
        &PublishTrainingPlanTool,
        None,
        json!({ "action": "publish" }),
    )
    .await;
    assert_eq!(refused["reason"], "own_plan", "{refused}");
    assert!(refused["error"]
        .as_str()
        .unwrap()
        .contains("Your own saves are already live"));

    let unknown = fx.coach_publishes(json!({ "action": "push" })).await;
    assert_eq!(unknown["reason"], "invalid_action", "{unknown}");
}
