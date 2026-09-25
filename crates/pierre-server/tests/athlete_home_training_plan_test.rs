// ABOUTME: GET /api/me/training-plan — what /plan shows, as the structured plan card, on the athlete's own today
// ABOUTME: Pins today against /plan today, rest vs uncovered days, the agent ladder, tenancy, locale and store failures

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home training-plan suite.
//!
//! Plans are seeded through the repository's own bundle save, the way
//! `save_training_plan` writes them, and the athlete's timezone is chosen so
//! their civil date is never the server's UTC one: a handler that projected
//! the plan on UTC today shows a different session and fails.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Days, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use pierre_commands::plan::PlanShowHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::periodization::{FlavourFamily, PhaseKind, Sequencing};
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::training_plans::PlanOwner;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_memory::training_plans::{
    FlavourSelection, GoalRace, PlanPhase, PlannedDay, RacePriority, SelectedBy,
};
use pierre_runtime_context::CommandCtx;
use pierre_services::plan_card::flavour_label;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

/// The flavour every seeded plan runs on.
const FLAVOUR_ID: &str = "polarized-classic";

struct Athlete {
    user: User,
    user_id: Uuid,
    tenant: TenantId,
    token: String,
}

async fn seed_athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user,
        user_id,
        tenant,
        token,
    }
}

/// A zone whose civil date is never UTC's at this hour: fourteen hours ahead
/// in the afternoon (UTC), twelve behind in the morning.
fn zone_off_utc_today() -> &'static str {
    if Utc::now().hour() >= 12 {
        "Etc/GMT-14"
    } else {
        "Etc/GMT+12"
    }
}

fn civil_today(zone: &str) -> NaiveDate {
    Utc::now()
        .with_timezone(&zone.parse::<Tz>().unwrap())
        .date_naive()
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn day(date: NaiveDate, sport: &str, workout: &str, minutes: Option<u32>) -> PlannedDay {
    PlannedDay {
        date: ymd(date),
        sport: sport.to_owned(),
        workout: workout.to_owned(),
        duration_min: minutes,
        intensity: if minutes.is_some() { "Z2" } else { "" }.to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
        template_source: None,
    }
}

/// A week of sessions from `start`, its fifth day a rest day, each session
/// named after its own date so no two days of the fixture read alike.
fn week(start: NaiveDate, label: &str) -> Vec<PlannedDay> {
    (0..7)
        .map(|i| {
            let date = start + Days::new(i);
            if i == 4 {
                day(date, "rest", "full rest", None)
            } else {
                day(
                    date,
                    "run",
                    &format!("{label} session {}", ymd(date)),
                    Some(45),
                )
            }
        })
        .collect()
}

/// Seed a plan under `agent` whose first week starts two days before
/// `today`, followed by `extra_weeks` more consecutive weeks.
async fn seed_plan(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tenant: TenantId,
    agent: Option<&str>,
    today: NaiveDate,
    extra_weeks: u64,
) {
    let goal = GoalRace {
        name: "Harricana 65".to_owned(),
        date: ymd(today + Days::new(60)),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    let start = today - Days::new(2);
    let phases = [PlanPhase {
        kind: PhaseKind::Build,
        start: ymd(start),
        weeks: 6,
        intent: "volume up".to_owned(),
        target_hours: Some(9.5),
        purpose: "aerobic durability".to_owned(),
        volume_share_of_peak: None,
        tid_target: None,
        hard_sessions_max: Some(2),
        session_mix: BTreeMap::new(),
        flavour_override: None,
        loading_pattern: None,
        skeleton_id: None,
    }];
    let flavour = FlavourSelection {
        id: FLAVOUR_ID.to_owned(),
        family: FlavourFamily::Polarized,
        sequencing: Sequencing::Linear,
        modifiers: Vec::new(),
        selected_by: SelectedBy::Coach,
        override_reason: Some("two hard days, as she is used to".to_owned()),
        verdict_snapshot: None,
        inputs_snapshot: None,
    };
    let starts: Vec<(String, Vec<PlannedDay>)> = (0..=extra_weeks)
        .map(|w| {
            let week_start = start + Days::new(7 * w);
            (ymd(week_start), week(week_start, &format!("week{w}")))
        })
        .collect();
    let weeks: Vec<PlanWeekInput<'_>> = starts
        .iter()
        .map(|(week_start, days)| PlanWeekInput {
            week_start,
            focus: "build volume",
            days,
            adjustment_reason: "",
            phase_index: Some(0),
        })
        .collect();
    resources
        .common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant.to_string(),
            user_id: &athlete.user_id.to_string(),
            owner: PlanOwner::from_slug(agent),
            goal_fact_id: None,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: Some(&[]),
                strategy: "rebuild volume then sharpen",
                phases: &phases,
                source_conversation_id: None,
                flavour: Some(&flavour),
                season_start: None,
                season_end: None,
            }),
            weeks: &weeks,
        })
        .await
        .unwrap();
}

async fn plan_json(
    resources: &Arc<ServerContext>,
    token: &str,
    query: &str,
) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .uri(format!("/api/me/training-plan{query}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn plan_ok(resources: &Arc<ServerContext>, token: &str, query: &str) -> Value {
    let (status, body) = plan_json(resources, token, query).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

/// The day card for `date` across every shown week, if any week lists it.
fn day_on<'a>(plan: &'a Value, date: &str) -> Option<&'a Value> {
    plan["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|week| week["days"].as_array().unwrap())
        .find(|day| day["date"] == date)
}

/// What `/plan today` answers for the athlete, outside any conversation.
async fn plan_command_today(resources: &Arc<ServerContext>, athlete: &Athlete) -> String {
    let ctx = PlatformCommandContext {
        user_id: athlete.user_id,
        tenant_id: athlete.tenant,
        channel_type: "telegram".to_owned(),
        args: vec!["today".to_owned()],
        raw_text: "/plan today".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: None,
        conversation_tenant_id: athlete.tenant,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(resources),
    };
    PlanShowHandler.execute(&ctx).await.unwrap().text
}

async fn set_timezone(resources: &Arc<ServerContext>, athlete: &Athlete, zone: &str) {
    resources
        .common
        .repos
        .users
        .set_timezone(athlete.user_id, zone)
        .await
        .unwrap();
}

/// An agent persona in the athlete's tenant, selected as their agent.
async fn select_agent(resources: &Arc<ServerContext>, athlete: &Athlete, title: &str) -> String {
    let repos = &resources.common.repos;
    let agent = repos
        .agents
        .create_system_agent(
            athlete.user_id,
            athlete.tenant,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "Test prompt".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Global,
            },
        )
        .await
        .unwrap()
        .id
        .to_string();
    repos
        .tenants
        .set_selected_agent(athlete.tenant, athlete.user_id, Some(&agent))
        .await
        .unwrap();
    agent
}

#[tokio::test]
async fn no_active_plan_is_null_on_the_athletes_today() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-none").await;
    let zone = zone_off_utc_today();
    set_timezone(&resources, &athlete, zone).await;

    let body = plan_ok(&resources, &athlete.token, "").await;
    let object = body.as_object().unwrap();
    assert!(
        object.contains_key("plan"),
        "the key is present even when null"
    );
    assert!(body["plan"].is_null());
    assert_eq!(body["today"], ymd(civil_today(zone)));
}

#[tokio::test]
async fn today_is_the_athletes_and_shows_the_session_plan_today_shows() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-today").await;
    let zone = zone_off_utc_today();
    set_timezone(&resources, &athlete, zone).await;
    let today = civil_today(zone);
    assert_ne!(
        today,
        Utc::now().date_naive(),
        "premise: the zone is off UTC's date"
    );
    seed_plan(&resources, &athlete, athlete.tenant, None, today, 0).await;

    let body = plan_ok(&resources, &athlete.token, "").await;
    assert_eq!(body["today"], ymd(today));
    let plan = &body["plan"];
    assert_eq!(plan["goal_race"]["name"], "Harricana 65");
    let session = day_on(plan, &ymd(today)).expect("today is in the current week");
    let workout = session["workout"].as_str().unwrap();
    assert_eq!(workout, format!("week0 session {}", ymd(today)));
    assert_eq!(session["rest"], false);
    assert_eq!(session["duration_min"], 45);

    let command = plan_command_today(&resources, &athlete).await;
    assert!(
        command.contains(workout),
        "/plan today must name the same session the page does: {command}"
    );
    let utc_session = format!("week0 session {}", ymd(Utc::now().date_naive()));
    assert!(
        !command.contains(&utc_session),
        "neither surface reads UTC's today: {command}"
    );

    let current: Vec<&Value> = plan["weeks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|week| week["current"] == true)
        .collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["week_start"], ymd(today - Days::new(2)));
    assert_eq!(plan["current_phase_index"], 0);
    assert_eq!(plan["phases"][0]["current"], true);
}

#[tokio::test]
async fn a_rest_day_is_a_rest_day_and_an_uncovered_date_is_in_no_week() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-rest").await;
    let today = Utc::now().date_naive();
    // Four weeks stored: the current and the next are shown, two deferred.
    seed_plan(&resources, &athlete, athlete.tenant, None, today, 3).await;

    let body = plan_ok(&resources, &athlete.token, "").await;
    let plan = &body["plan"];
    assert_eq!(plan["weeks"].as_array().unwrap().len(), 2);
    assert_eq!(plan["weeks_deferred"], 2);

    let rest_date = ymd(today + Days::new(2));
    let rest = day_on(plan, &rest_date).expect("the rest day is listed");
    assert_eq!(rest["rest"], true);
    assert_eq!(rest["workout"], "full rest");
    assert!(rest.get("duration_min").is_none());

    let next_week_session = day_on(plan, &ymd(today + Days::new(6))).unwrap();
    assert_eq!(next_week_session["rest"], false);

    let uncovered = ymd(today + Days::new(40));
    assert!(
        day_on(plan, &uncovered).is_none(),
        "a date no shown week spans is absent, never a rest day"
    );
}

#[tokio::test]
async fn the_plan_is_the_one_the_selected_agent_holds() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-agent").await;
    let today = Utc::now().date_naive();
    let agent = select_agent(&resources, &athlete, "Trail Coach").await;
    seed_plan(&resources, &athlete, athlete.tenant, Some(&agent), today, 0).await;

    let body = plan_ok(&resources, &athlete.token, "").await;
    assert_eq!(body["plan"]["goal_race"]["name"], "Harricana 65");

    // Once another agent is selected, the first agent's plan is not theirs.
    select_agent(&resources, &athlete, "Road Coach").await;
    let body = plan_ok(&resources, &athlete.token, "").await;
    assert!(body["plan"].is_null());
}

#[tokio::test]
async fn another_tenants_plan_is_invisible() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-tenancy").await;
    let stranger = seed_athlete(&resources, "plan-stranger").await;
    let today = Utc::now().date_naive();
    seed_plan(&resources, &athlete, athlete.tenant, None, today, 0).await;

    let elsewhere = TenantId::generate();
    resources
        .common
        .repos
        .tenants
        .create(&Tenant {
            id: elsewhere,
            name: "second tenant".to_owned(),
            slug: format!("second-{elsewhere}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: athlete.user_id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let elsewhere_token = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &athlete.user,
            &resources.auth.jwks_manager,
            Some(elsewhere.to_string()),
        )
        .unwrap();

    assert!(!plan_ok(&resources, &athlete.token, "").await["plan"].is_null());
    assert!(plan_ok(&resources, &elsewhere_token, "").await["plan"].is_null());
    assert!(plan_ok(&resources, &stranger.token, "").await["plan"].is_null());
}

#[tokio::test]
async fn the_flavour_label_follows_the_requested_then_the_stored_locale() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-locale").await;
    let today = Utc::now().date_naive();
    seed_plan(&resources, &athlete, athlete.tenant, None, today, 0).await;
    resources
        .common
        .repos
        .users
        .update_locale(athlete.user_id, "de")
        .await
        .unwrap();
    let registry = &resources.mcp.messaging_strings_registry;
    let label = |locale: &str| flavour_label(registry, locale, FLAVOUR_ID);
    assert_ne!(label("fr"), label("en"), "premise: the label is translated");

    for (query, locale) in [
        ("?locale=fr", "fr"),
        ("?locale=en", "en"),
        ("?locale=xx", "de"),
        ("", "de"),
    ] {
        let body = plan_ok(&resources, &athlete.token, query).await;
        assert_eq!(body["plan"]["flavour"]["id"], FLAVOUR_ID);
        assert_eq!(body["plan"]["flavour"]["label"], label(locale), "{query}");
    }
}

#[tokio::test]
async fn an_unreadable_plan_store_is_a_server_error_not_an_empty_plan() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "plan-broken").await;
    seed_plan(
        &resources,
        &athlete,
        athlete.tenant,
        None,
        Utc::now().date_naive(),
        0,
    )
    .await;
    // The weeks table going away mid-read is the failure: the plan row reads,
    // its weeks do not.
    match resources.agent.database.as_ref() {
        Database::SQLite(sqlite) => {
            sqlx::query("DROP TABLE training_plan_weeks")
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(postgres) => {
            sqlx::query("DROP TABLE training_plan_weeks CASCADE")
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }

    let (status, body) = plan_json(&resources, &athlete.token, "").await;
    assert!(
        status.is_server_error(),
        "a store that cannot be read must not read as no plan: {status} {body}"
    );
    assert!(body.get("plan").is_none(), "{body}");
}
