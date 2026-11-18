// ABOUTME: Response date goldens for the athlete Home routes and the training plan tool
// ABOUTME: Pins the calendar's days in the athlete's zone, UTC starts written with Z, plan dates, and how from and to are read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/me/calendar`, `/api/me/activities/recent`,
//! `/api/me/activities/{provider}/{id}`, `/api/me/training-plan`, and the
//! `get_training_plan` tool (carnet#1620, gate 3).
//!
//! Workouts are written to the cache and the plan through the repository's
//! bundle save, at instants and days the suite chooses; each route is called
//! through `athlete_home_routes()` and every date in its body is compared
//! with a literal table.
//!
//! The calendar and the recent list read only the last 180 days of the
//! cache, so their workouts are dated from today: the times of day, their
//! fractions and the day each falls on in the athlete's zone are pinned, the
//! dates themselves are today's. The plan's dates and the activity view's
//! are literal.
//!
//! Stamped with the wall clock, and so compared by shape only here:
//!
//! - the calendar: `today` and `history_start`;
//! - `/api/me/training-plan`: `today`;
//! - a refusal's `timestamp`, whose shape keeps the clock's fraction.
//!
//! The values of `today` and `history_start`, and a plan dated from today
//! whose current week renders, are in `date_response_today_golden_test`. The
//! plan's own `created_at` and `updated_at` are moved to chosen seconds and
//! compared as literals.

#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{ActivityBuilder, SportType};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, epoch, get, instant, mcp_call, mcp_payload, on_the_clock,
    restamp_seconds, seed_athlete, send, table, tool_payload, utc_days_before_today, with_values,
    Athlete,
};

const TORONTO: &str = "America/Toronto";
const KOLKATA: &str = "Asia/Kolkata";

async fn cache(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    rows: &[(&str, SportType, String)],
) {
    let rows: Vec<_> = rows
        .iter()
        .map(|(id, sport, start)| {
            ActivityBuilder::new(*id, *id, sport.clone(), instant(start), 1_800, UPLOAD)
                .distance_meters(5_000.0)
                .build()
        })
        .collect();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, UPLOAD, &rows)
        .await
        .unwrap();
}

/// The calendar and the recent list read only the last 180 days of the
/// cache, so their workouts are dated from today: the days cannot be chosen,
/// the times of day and their fractions can. `days` are UTC dates, the
/// newest first; a ride starts at 02:59:03.500250 on the first, a swim at
/// 10:15:30.250125 on the second and a run at 12:00:00 on the third.
async fn cache_recent_workouts(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    days: &[String; 3],
) {
    let [ride, swim, run] = days;
    cache(
        resources,
        athlete,
        &[
            (
                "recent-ride",
                SportType::Ride,
                format!("{ride}T02:59:03.500250"),
            ),
            (
                "recent-swim",
                SportType::Swim,
                format!("{swim}T10:15:30.250125"),
            ),
            ("recent-run", SportType::Run, format!("{run}T12:00:00")),
        ],
    )
    .await;
}

fn planned(date: &str) -> PlannedDay {
    PlannedDay {
        date: date.to_owned(),
        sport: "run".to_owned(),
        workout: "easy run".to_owned(),
        duration_min: Some(45),
        intensity: "Z2".to_owned(),
        steps: Vec::new(),
        fueling: None,
        template_slug: None,
        template_params: None,
        template_source: None,
        template_preset: None,
        envelope: None,
    }
}

/// A plan for a race on 2026-10-24: one three-week phase from Monday
/// 2026-08-17, and the week of the 24th with sessions on the 24th and 27th.
async fn save_plan(resources: &Arc<ServerContext>, athlete: &Athlete) {
    let goal = GoalRace {
        name: "Harricana 65".to_owned(),
        date: "2026-10-24".to_owned(),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    let phases = [PlanPhase {
        kind: PhaseKind::Build,
        start: "2026-08-17".to_owned(),
        weeks: 3,
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
    let days = [planned("2026-08-24"), planned("2026-08-27")];
    let weeks = [PlanWeekInput {
        week_start: "2026-08-24",
        focus: "build volume",
        days: &days,
        adjustment_reason: "",
        phase_index: Some(0),
    }];
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

async fn home(resources: &Arc<ServerContext>, athlete: &Athlete, uri: &str) -> (StatusCode, Value) {
    send(
        athlete_home_routes().with_state(Arc::clone(resources)),
        get(uri, &athlete.token),
    )
    .await
}

async fn home_ok(resources: &Arc<ServerContext>, athlete: &Athlete, uri: &str) -> Value {
    let (status, body) = home(resources, athlete, uri).await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

/// The calendar of the seven UTC days up to today.
async fn last_week(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    let [to, from] = utc_days_before_today([0, 6]);
    home_ok(
        resources,
        athlete,
        &format!("/api/me/calendar?from={from}&to={to}"),
    )
    .await
}

#[tokio::test]
async fn the_calendar_in_toronto_puts_each_start_on_the_athletes_day() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-cal-toronto", Some(TORONTO), "en").await;
        let [today, _, two, three, four, _, six] = utc_days_before_today([0, 1, 2, 3, 4, 5, 6]);
        cache_recent_workouts(
            &resources,
            &athlete,
            &[two.clone(), three.clone(), four.clone()],
        )
        .await;

        let body = last_week(&resources, &athlete).await;

        // 02:59 UTC is the evening before in Toronto, summer or winter: the ride
        // of `two` is on `three`, the swim's day.
        assert_eq!(
            // `today` and `history_start` by shape here; their values are
            // compared in date_response_today_golden_test.
            dates_in_stamped(&body, &["/today", "/history_start"]),
            table(&[
                (
                    "/activities/0/activity/start_date",
                    format!("{four}T12:00:00Z").as_str()
                ),
                ("/activities/0/date", four.as_str()),
                (
                    "/activities/1/activity/start_date",
                    format!("{three}T10:15:30.250125Z").as_str()
                ),
                ("/activities/1/date", three.as_str()),
                (
                    "/activities/2/activity/start_date",
                    format!("{two}T02:59:03.500250Z").as_str()
                ),
                ("/activities/2/date", three.as_str()),
                ("/from", six.as_str()),
                ("/history_start", "dddd-dd-dd"),
                ("/to", today.as_str()),
                ("/today", "dddd-dd-dd"),
            ]),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_calendar_in_kolkata_puts_the_same_starts_on_other_days() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-cal-kolkata", Some(KOLKATA), "en").await;
        let [today, _, two, three, four, _, six] = utc_days_before_today([0, 1, 2, 3, 4, 5, 6]);
        cache_recent_workouts(
            &resources,
            &athlete,
            &[two.clone(), three.clone(), four.clone()],
        )
        .await;

        let body = last_week(&resources, &athlete).await;

        // 02:59 UTC is 08:29 the same day in Kolkata.
        assert_eq!(
            // `today` and `history_start` by shape here; their values are
            // compared in date_response_today_golden_test.
            dates_in_stamped(&body, &["/today", "/history_start"]),
            table(&[
                (
                    "/activities/0/activity/start_date",
                    format!("{four}T12:00:00Z").as_str()
                ),
                ("/activities/0/date", four.as_str()),
                (
                    "/activities/1/activity/start_date",
                    format!("{three}T10:15:30.250125Z").as_str()
                ),
                ("/activities/1/date", three.as_str()),
                (
                    "/activities/2/activity/start_date",
                    format!("{two}T02:59:03.500250Z").as_str()
                ),
                ("/activities/2/date", two.as_str()),
                ("/from", six.as_str()),
                ("/history_start", "dddd-dd-dd"),
                ("/to", today.as_str()),
                ("/today", "dddd-dd-dd"),
            ]),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_calendar_answers_a_plans_week_and_its_days_as_calendar_dates() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-cal-plan", Some(TORONTO), "en").await;
        save_plan(&resources, &athlete).await;

        let body = home_ok(
            &resources,
            &athlete,
            "/api/me/calendar?from=2026-08-24&to=2026-08-30",
        )
        .await;

        assert_eq!(
            // `today` and `history_start` by shape here; their values are
            // compared in date_response_today_golden_test.
            dates_in_stamped(&body, &["/today", "/history_start"]),
            table(&[
                ("/from", "2026-08-24"),
                ("/history_start", "dddd-dd-dd"),
                ("/plan_weeks/0/days/0/date", "2026-08-24"),
                ("/plan_weeks/0/days/1/date", "2026-08-27"),
                ("/plan_weeks/0/week_start", "2026-08-24"),
                ("/to", "2026-08-30"),
                ("/today", "dddd-dd-dd"),
            ]),
            "{body}"
        );
    })
    .await;
}

/// The two bounds a read echoes, when it is not refused.
type Echoed = Option<(&'static str, &'static str)>;

#[tokio::test]
async fn the_calendar_reads_from_and_to_as_calendar_dates_only() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-cal-inbound", Some(TORONTO), "en").await;

    // (from, to, status, the `from` and `to` echoed when it is read).
    let read: [(&str, &str, u16, Echoed); 10] = [
        (
            "2026-08-24",
            "2026-08-30",
            200,
            Some(("2026-08-24", "2026-08-30")),
        ),
        // Digits without their padding are read, and echoed padded.
        (
            "2026-8-24",
            "2026-8-30",
            200,
            Some(("2026-08-24", "2026-08-30")),
        ),
        (
            "2026-08-24",
            "2026-08-24",
            200,
            Some(("2026-08-24", "2026-08-24")),
        ),
        // An instant, a compact date, a day-first date, a day that does not
        // exist, a missing bound: refused.
        ("2026-08-24T00:00:00Z", "2026-08-30", 400, None),
        ("20260824", "2026-08-30", 400, None),
        ("24/08/2026", "2026-08-30", 400, None),
        ("2026-02-30", "2026-03-02", 400, None),
        ("2026-08-24", "", 400, None),
        // Backwards, and past the span a read may cover.
        ("2026-08-30", "2026-08-24", 400, None),
        ("2026-01-01", "2026-12-31", 400, None),
    ];
    for (from, to, status, echoed) in read {
        let uri = format!(
            "/api/me/calendar?from={}&to={}",
            urlencoding::encode(from),
            urlencoding::encode(to)
        );
        let (answered, body) = home(&resources, &athlete, &uri).await;
        assert_eq!(answered.as_u16(), status, "{uri}: {body}");
        if let Some((from, to)) = echoed {
            assert_eq!(
                (body["from"].as_str(), body["to"].as_str()),
                (Some(from), Some(to)),
                "the bounds echoed for {uri}: {body}"
            );
        }
    }
}

#[tokio::test]
async fn a_refused_calendar_read_is_stamped_with_an_offset() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-cal-refused", Some(TORONTO), "en").await;

        let (status, body) = home(
            &resources,
            &athlete,
            "/api/me/calendar?from=2026-08-30&to=2026-08-24",
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            dates_in_stamped(&body, &["/timestamp"]),
            table(&[("/timestamp", "dddd-dd-ddTdd:dd:dd.f+dd:dd")]),
            "{body}"
        );
    })
    .await;
}

/// The activity view reads one cached row by its id, however old.
#[tokio::test]
async fn the_activity_view_writes_the_start_in_utc_with_z() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-view", Some(TORONTO), "en").await;
    cache(
        &resources,
        &athlete,
        &[(
            "u-ride",
            SportType::Ride,
            "2026-08-28T02:59:03.500250".to_owned(),
        )],
    )
    .await;

    let view = home_ok(&resources, &athlete, "/api/me/activities/upload/u-ride").await;

    assert_eq!(
        dates_in(&view),
        table(&[("/activity/start_date", "2026-08-28T02:59:03.500250Z")]),
        "{view}"
    );
}

#[tokio::test]
async fn the_recent_list_writes_starts_in_utc_with_z() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-recent", Some(TORONTO), "en").await;
    let days = utc_days_before_today([1, 2, 3]);
    cache_recent_workouts(&resources, &athlete, &days).await;
    let [first, second, third] = days;

    let recent = home_ok(&resources, &athlete, "/api/me/activities/recent").await;

    assert_eq!(
        dates_in(&recent),
        table(&[
            (
                "/activities/0/start_date",
                format!("{first}T02:59:03.500250Z").as_str()
            ),
            (
                "/activities/1/start_date",
                format!("{second}T10:15:30.250125Z").as_str()
            ),
            (
                "/activities/2/start_date",
                format!("{third}T12:00:00Z").as_str()
            ),
        ]),
        "{recent}"
    );
}

#[tokio::test]
async fn the_training_plan_route_answers_the_plans_days_and_each_phases_end() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-plan-route", Some(TORONTO), "en").await;
        save_plan(&resources, &athlete).await;

        let body = home_ok(&resources, &athlete, "/api/me/training-plan").await;

        // The phase's end is computed: three weeks after its start. A phase and
        // a week wholly in the past are not current, and the week is not listed.
        assert_eq!(
            with_values(
                dates_in_stamped(&body, &["/today"]),
                &body,
                &[
                    "/plan/phases/0/current",
                    "/plan/weeks",
                    "/plan/weeks_deferred"
                ]
            ),
            table(&[
                ("/plan/goal_race/date", "2026-10-24"),
                ("/plan/phases/0/current", "false"),
                ("/plan/phases/0/end", "2026-09-07"),
                ("/plan/phases/0/start", "2026-08-17"),
                ("/plan/weeks", "[]"),
                ("/plan/weeks_deferred", "0"),
                ("/today", "dddd-dd-dd"),
            ]),
            "{body}"
        );
    })
    .await;
}

const MOVE_PLAN: &str = "UPDATE training_plans SET created_at = $1 \
                         WHERE user_id = $2 AND CAST(tenant_id AS TEXT) = $3";
const TOUCH_PLAN: &str = "UPDATE training_plans SET updated_at = $1 \
                          WHERE user_id = $2 AND CAST(tenant_id AS TEXT) = $3";

#[tokio::test]
async fn the_training_plan_tool_stamps_the_plan_in_whole_seconds_with_z() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-plan-tool", Some(TORONTO), "en").await;
    save_plan(&resources, &athlete).await;
    // The plan's stamps are whole seconds since the epoch.
    let database = &resources.agent.database;
    let user = athlete.user_id.to_string();
    let tenant = Some(athlete.tenant);
    restamp_seconds(
        database,
        MOVE_PLAN,
        epoch("2026-08-16T23:59:03"),
        &user,
        tenant,
    )
    .await;
    restamp_seconds(
        database,
        TOUCH_PLAN,
        epoch("2026-08-28T02:59:03"),
        &user,
        tenant,
    )
    .await;

    let payload = tool_payload(&resources, &athlete, "get_training_plan", json!({})).await;
    // And as an MCP client receives it, through `POST /mcp`.
    let envelope = mcp_call(&resources, &athlete, "get_training_plan", json!({})).await;
    assert_eq!(
        dates_in(&mcp_payload(&envelope)),
        dates_in(&payload),
        "{envelope}"
    );

    // No fraction at all, where every other instant the tools answer keeps
    // the clock's.
    assert_eq!(
        dates_in(&payload),
        table(&[
            ("/plan/created_at", "2026-08-16T23:59:03Z"),
            ("/plan/goal_race/date", "2026-10-24"),
            ("/plan/phases/0/start", "2026-08-17"),
            ("/plan/updated_at", "2026-08-28T02:59:03Z"),
        ]),
        "{payload}"
    );
}
