// ABOUTME: GET /api/me/calendar — cached workouts on the athlete's days and the plan's weeks over a span, for Home's paging week strip
// ABOUTME: Pins the athlete-zone day, the merge, tenancy, the plan's past and future weeks, null without a plan, and the span bound

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete Home calendar suite.
//!
//! Workouts are seeded through the activity cache's own writer and plans
//! through the repository's bundle save, the way `save_training_plan` writes
//! them, and the route's JSON is read back: a handler that bucketed by UTC,
//! leaked another tenant's rows or projected only the card's two weeks fails
//! on content.
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so
// without a surviving crate doc the command-line `-D warnings` trips
// `missing_docs`.
#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{Activity, ActivityBuilder, SportType, Tenant, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

/// The athlete's zone in every test but the UTC one: five hours behind UTC in
/// summer, so an evening run lands on the next UTC day.
const TORONTO: &str = "America/Toronto";

struct Athlete {
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
    resources
        .common
        .repos
        .users
        .set_timezone(user_id, TORONTO)
        .await
        .unwrap();
    Athlete {
        user_id,
        tenant,
        token,
    }
}

async fn calendar_json(
    resources: &Arc<ServerContext>,
    token: &str,
    query: &str,
) -> (StatusCode, Value) {
    let response = athlete_home_routes()
        .with_state(Arc::clone(resources))
        .oneshot(
            Request::builder()
                .uri(format!("/api/me/calendar{query}"))
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

async fn calendar(
    resources: &Arc<ServerContext>,
    token: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Value {
    let (status, body) = calendar_json(
        resources,
        token,
        &format!("?from={}&to={}", ymd(from), ymd(to)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn toronto_today() -> NaiveDate {
    Utc::now()
        .with_timezone(&TORONTO.parse::<Tz>().unwrap())
        .date_naive()
}

/// `hour:00` on `date` in Toronto, as the instant a provider stores.
fn toronto_at(date: NaiveDate, hour: u32) -> DateTime<Utc> {
    TORONTO
        .parse::<Tz>()
        .unwrap()
        .from_local_datetime(&date.and_hms_opt(hour, 0, 0).unwrap())
        .earliest()
        .unwrap()
        .with_timezone(&Utc)
}

fn run(id: &str, provider: &str, started: DateTime<Utc>) -> Activity {
    ActivityBuilder::new(
        id,
        format!("Run {id}"),
        SportType::Run,
        started,
        2_400,
        provider,
    )
    .distance_meters(8_000.0)
    .build()
}

async fn cache(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    provider: &str,
    rows: &[Activity],
) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, provider, rows)
        .await
        .unwrap();
}

/// `(date, activity id)` of every workout the calendar answers, in its order.
fn placed(body: &Value) -> Vec<(String, String)> {
    body["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["date"].as_str().unwrap().to_owned(),
                row["activity"]["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

/// A session on every day of the week from `start`, named after its date.
fn week_days(start: NaiveDate) -> Vec<PlannedDay> {
    (0..7)
        .map(|i| {
            let date = start + Days::new(i);
            PlannedDay {
                date: ymd(date),
                sport: "run".to_owned(),
                workout: format!("session {}", ymd(date)),
                duration_min: Some(45),
                intensity: "Z2".to_owned(),
                steps: Vec::new(),
                fueling: None,
                template_slug: None,
                template_params: None,
                template_source: None,
            }
        })
        .collect()
}

/// A plan whose weeks run from three weeks before this week's Monday to
/// three weeks after it — seven weeks, where the card shows two.
async fn seed_plan(resources: &Arc<ServerContext>, athlete: &Athlete, monday: NaiveDate) {
    let first = monday - Days::new(21);
    let goal = GoalRace {
        name: "Harricana 65".to_owned(),
        date: ymd(monday + Days::new(60)),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    let phases = [PlanPhase {
        kind: PhaseKind::Build,
        start: ymd(first),
        weeks: 7,
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
    let starts: Vec<(String, Vec<PlannedDay>)> = (0..7)
        .map(|w| {
            let start = first + Days::new(7 * w);
            (ymd(start), week_days(start))
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
            tenant_id: &athlete.tenant.to_string(),
            user_id: &athlete.user_id.to_string(),
            author: PlanAuthor::from_agent(None),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(PlanOutlineInput {
                goal_race: &goal,
                races: Some(&[]),
                strategy: "rebuild volume then sharpen",
                phases: &phases,
                source_conversation_id: None,
                flavour: None,
                season_start: None,
                season_end: None,
            }),
            weeks: &weeks,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
}

fn monday_of(date: NaiveDate) -> NaiveDate {
    date - Days::new(u64::from(date.weekday().num_days_from_monday()))
}

#[tokio::test]
async fn workouts_land_on_the_athletes_own_day_oldest_first() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "calendar-days").await;
    let today = toronto_today();
    let monday = monday_of(today) - Days::new(7);
    // 21:00 Toronto is the next UTC day: a UTC bucket puts it a day late.
    let rows = [
        run("evening", "strava", toronto_at(monday + Days::new(1), 21)),
        run("morning", "strava", toronto_at(monday + Days::new(3), 7)),
        run("next-week", "strava", toronto_at(monday + Days::new(7), 7)),
    ];
    cache(&resources, athlete.user_id, athlete.tenant, "strava", &rows).await;

    let body = calendar(&resources, &athlete.token, monday, monday + Days::new(6)).await;
    assert_eq!(
        placed(&body),
        [
            (ymd(monday + Days::new(1)), "evening".to_owned()),
            (ymd(monday + Days::new(3)), "morning".to_owned()),
        ]
    );
    assert_eq!(body["today"], ymd(today));
    assert_eq!(body["from"], ymd(monday));
    assert_eq!(body["to"], ymd(monday + Days::new(6)));
    let row = &body["activities"][0]["activity"];
    assert_eq!(row["provider"], "strava");
    assert_eq!(row["name"], "Run evening");
    assert_eq!(row["duration_seconds"], 2_400);
}

#[tokio::test]
async fn one_workout_synced_to_two_providers_is_one_entry() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "calendar-merge").await;
    let day = toronto_today() - Days::new(2);
    let start = toronto_at(day, 8);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run("s-ride", "strava", start)],
    )
    .await;
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "garmin",
        &[run("g-ride", "garmin", start)],
    )
    .await;

    let body = calendar(&resources, &athlete.token, day, day).await;
    assert_eq!(body["activities"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["activities"][0]["date"], ymd(day));
}

#[tokio::test]
async fn another_tenants_and_another_users_workouts_are_invisible() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "calendar-mine").await;
    let stranger = seed_athlete(&resources, "calendar-theirs").await;
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
    let day = toronto_today() - Days::new(1);
    cache(
        &resources,
        athlete.user_id,
        athlete.tenant,
        "strava",
        &[run("mine", "strava", toronto_at(day, 9))],
    )
    .await;
    cache(
        &resources,
        athlete.user_id,
        elsewhere,
        "strava",
        &[run("elsewhere", "strava", toronto_at(day, 10))],
    )
    .await;
    cache(
        &resources,
        stranger.user_id,
        stranger.tenant,
        "strava",
        &[run("theirs", "strava", toronto_at(day, 11))],
    )
    .await;

    let body = calendar(&resources, &athlete.token, day, day).await;
    assert_eq!(placed(&body), [(ymd(day), "mine".to_owned())]);
}

#[tokio::test]
async fn no_active_plan_is_null_and_a_plan_answers_its_past_and_future_weeks() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "calendar-plan").await;
    let monday = monday_of(toronto_today());
    let from = monday - Days::new(14);
    let to = monday + Days::new(20);

    let before = calendar(&resources, &athlete.token, from, to).await;
    assert!(before.as_object().unwrap().contains_key("plan_weeks"));
    assert!(before["plan_weeks"].is_null(), "{before}");

    seed_plan(&resources, &athlete, monday).await;
    let body = calendar(&resources, &athlete.token, from, to).await;
    let weeks = body["plan_weeks"].as_array().unwrap();
    let starts: Vec<&str> = weeks
        .iter()
        .map(|w| w["week_start"].as_str().unwrap())
        .collect();
    assert_eq!(
        starts,
        [
            ymd(monday - Days::new(14)),
            ymd(monday - Days::new(7)),
            ymd(monday),
            ymd(monday + Days::new(7)),
            ymd(monday + Days::new(14)),
        ]
    );
    let current: Vec<bool> = weeks
        .iter()
        .map(|w| w["current"].as_bool().unwrap())
        .collect();
    assert_eq!(current, [false, false, true, false, false]);
    assert_eq!(
        weeks[0]["days"][0]["workout"],
        format!("session {}", ymd(from))
    );
    assert_eq!(weeks[0]["days"][0]["duration_min"], 45);
}

#[tokio::test]
async fn nothing_is_answered_before_history_starts() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "calendar-edge").await;
    let body = calendar(&resources, &athlete.token, toronto_today(), toronto_today()).await;
    let history_start =
        NaiveDate::parse_from_str(body["history_start"].as_str().unwrap(), "%Y-%m-%d").unwrap();
    assert!(history_start < toronto_today());

    // A row the prune has not reached yet, on the day the cutoff falls in, is
    // part of a day the cache no longer holds in full.
    let partial = history_start - Days::new(1);
    let rows = [
        run("partial", "strava", toronto_at(partial, 23)),
        run("kept", "strava", toronto_at(history_start, 12)),
    ];
    cache(&resources, athlete.user_id, athlete.tenant, "strava", &rows).await;
    let body = calendar(&resources, &athlete.token, partial, history_start).await;
    assert_eq!(placed(&body), [(ymd(history_start), "kept".to_owned())]);
}

#[tokio::test]
async fn a_span_longer_than_six_weeks_or_reversed_is_refused() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "calendar-span").await;
    let from = toronto_today();
    let (status, _) = calendar_json(
        &resources,
        &athlete.token,
        &format!("?from={}&to={}", ymd(from), ymd(from + Days::new(42))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = calendar_json(
        &resources,
        &athlete.token,
        &format!("?from={}&to={}", ymd(from), ymd(from - Days::new(1))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) =
        calendar_json(&resources, &athlete.token, "?from=2026-02-30&to=2026-03-01").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
