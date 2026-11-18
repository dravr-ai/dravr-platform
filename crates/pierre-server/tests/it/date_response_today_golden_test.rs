// ABOUTME: Response date goldens for the athlete's own "today", compared by value in two zones a day apart
// ABOUTME: Pins the calendar, the plan dated from today, the activity list anchor and the weekly volume on the athlete's day
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: what a handler derives from the wall clock and the
//! athlete's zone (carnet#1620, gate 3).
//!
//! Production reads the clock itself, so "today" cannot be chosen. It can be
//! known: each test seeds one athlete in `Pacific/Kiritimati` (+14:00) and
//! one in `Etc/GMT+12` (-12:00), two zones whose offsets never change and
//! whose days always differ, and compares `today` with the day computed from
//! one reading of the clock. A handler that answers UTC's day, or a day off,
//! is wrong for at least one of them at any hour. A test whose request
//! straddles a midnight in either zone runs again.
//!
//! - `/api/me/calendar`: `today`, `history_start` and the day each start
//!   falls on;
//! - `/api/me/training-plan` and `get_training_plan`: a plan dated from the
//!   athlete's today, so the current week, its days and the phase's
//!   `current` flag render;
//! - `get_activities`: the `[Today]` anchor and `start_date_local` at both
//!   offsets;
//! - `/api/me/training-volume`: `today` and each `week_start`.

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
    dates_in, day, epoch, get, instant, monday_of, restamp_seconds, seed_athlete, send, shifted,
    table, tool_payload, weekday_word, with_values, Athlete, Today, KIRITIMATI,
    WEST_OF_THE_DATE_LINE,
};

const ZONES: [&str; 2] = [KIRITIMATI, WEST_OF_THE_DATE_LINE];

const MOVE_PLAN: &str = "UPDATE training_plans SET created_at = $1, updated_at = $1 \
                         WHERE user_id = $2 AND CAST(tenant_id AS TEXT) = $3";
/// One week's row, named by its Monday and its athlete.
const MOVE_WEEK: &str = "UPDATE training_plan_weeks SET created_at = $1, updated_at = $1 \
                         WHERE week_start || user_id = $2 AND CAST(tenant_id AS TEXT) = $3";

/// One ride at 11:30:00.500250 UTC the day before today's UTC date: 01:30 the
/// next day in Kiritimati, 23:30 the day before in the zone west of the line.
async fn cache_a_ride(resources: &Arc<ServerContext>, athlete: &Athlete, today: &Today) {
    let ride = ActivityBuilder::new(
        "zoned-ride",
        "Ride",
        SportType::Ride,
        instant(&format!("{}T11:30:00.500250", shifted(today.utc, -1))),
        3_600,
        UPLOAD,
    )
    .distance_meters(30_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, UPLOAD, &[ride])
        .await
        .unwrap();
}

/// The day the ride falls on in `zone`, as days from today's UTC date.
fn ride_day(today: &Today, zone: &str) -> String {
    shifted(today.utc, if zone == KIRITIMATI { 0 } else { -2 })
}

/// The ride's wall-clock time and offset in `zone`.
fn ride_clock(zone: &str) -> (&'static str, &'static str) {
    if zone == KIRITIMATI {
        ("01:30", "+14:00")
    } else {
        ("23:30", "-12:00")
    }
}

async fn home(resources: &Arc<ServerContext>, athlete: &Athlete, uri: &str) -> Value {
    let (status, body) = send(
        athlete_home_routes().with_state(Arc::clone(resources)),
        get(uri, &athlete.token),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    body
}

#[tokio::test]
async fn the_calendar_answers_the_athletes_today_and_where_its_history_starts() {
    let resources = common::create_test_server_resources().await.unwrap();
    for zone in ZONES {
        let (today, body) = loop {
            let today = Today::read();
            let athlete = seed_athlete(&resources, "golden-today-cal", Some(zone), "en").await;
            cache_a_ride(&resources, &athlete, &today).await;
            let day = today.in_zone(zone);
            let uri = format!(
                "/api/me/calendar?from={}&to={}",
                shifted(day, -6),
                shifted(day, 0)
            );
            let body = home(&resources, &athlete, &uri).await;
            if today.holds() {
                break (today, body);
            }
        };
        let day = today.in_zone(zone);

        // The cache is read 180 days back: history starts 179 days before
        // the athlete's today.
        assert_eq!(
            dates_in(&body),
            table(&[
                (
                    "/activities/0/activity/start_date",
                    format!("{}T11:30:00.500250Z", shifted(today.utc, -1)).as_str()
                ),
                ("/activities/0/date", ride_day(&today, zone).as_str()),
                ("/from", shifted(day, -6).as_str()),
                ("/history_start", shifted(day, -179).as_str()),
                ("/to", shifted(day, 0).as_str()),
                ("/today", shifted(day, 0).as_str()),
            ]),
            "{zone}: {body}"
        );
    }
}

fn planned(date: String) -> PlannedDay {
    PlannedDay {
        date,
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

/// A plan around the athlete's own week: a three-week phase that opened the
/// Monday before this week's, this week with sessions on its Monday and
/// Thursday, next week with one on its Wednesday, and a race 61 days after
/// this week's Monday.
async fn save_plan_around(resources: &Arc<ServerContext>, athlete: &Athlete, monday: &str) {
    let monday = day(monday);
    let goal = GoalRace {
        name: "Harricana 65".to_owned(),
        date: shifted(monday, 61),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    let phases = [PlanPhase {
        kind: PhaseKind::Build,
        start: shifted(monday, -7),
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
    let this_week = [planned(shifted(monday, 0)), planned(shifted(monday, 3))];
    let next_week = [planned(shifted(monday, 9))];
    let (this_start, next_start) = (shifted(monday, 0), shifted(monday, 7));
    let weeks = [
        PlanWeekInput {
            week_start: &this_start,
            focus: "build volume",
            days: &this_week,
            adjustment_reason: "",
            phase_index: Some(0),
        },
        PlanWeekInput {
            week_start: &next_start,
            focus: "hold",
            days: &next_week,
            adjustment_reason: "",
            phase_index: Some(0),
        },
    ];
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

#[tokio::test]
async fn a_plan_dated_from_today_renders_its_current_week_on_the_athletes_calendar() {
    let resources = common::create_test_server_resources().await.unwrap();
    for zone in ZONES {
        let (today, route, tool) = loop {
            let today = Today::read();
            let athlete = seed_athlete(&resources, "golden-today-plan", Some(zone), "en").await;
            let monday = shifted(monday_of(today.in_zone(zone)), 0);
            save_plan_around(&resources, &athlete, &monday).await;
            // The plan and each week are stamped in whole seconds since the
            // epoch: moved to chosen ones.
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
                MOVE_WEEK,
                epoch("2026-08-17T06:00:00"),
                &format!("{monday}{user}"),
                tenant,
            )
            .await;
            restamp_seconds(
                database,
                MOVE_WEEK,
                epoch("2026-08-18T07:30:59"),
                &format!("{}{user}", shifted(day(&monday), 7)),
                tenant,
            )
            .await;
            let route = home(&resources, &athlete, "/api/me/training-plan").await;
            let tool = tool_payload(&resources, &athlete, "get_training_plan", json!({})).await;
            if today.holds() {
                break (today, route, tool);
            }
        };
        let day = today.in_zone(zone);
        let monday = monday_of(day);

        // The phase opened last Monday and runs three weeks: it is current,
        // and it ends the Monday two weeks after this one.
        assert_eq!(
            with_values(
                dates_in(&route),
                &route,
                &["/plan/phases/0/current", "/plan/weeks_deferred"]
            ),
            table(&[
                ("/plan/goal_race/date", shifted(monday, 61).as_str()),
                ("/plan/phases/0/current", "true"),
                ("/plan/phases/0/end", shifted(monday, 14).as_str()),
                ("/plan/phases/0/start", shifted(monday, -7).as_str()),
                ("/plan/weeks/0/days/0/date", shifted(monday, 0).as_str()),
                ("/plan/weeks/0/days/1/date", shifted(monday, 3).as_str()),
                ("/plan/weeks/0/week_start", shifted(monday, 0).as_str()),
                ("/plan/weeks/1/days/0/date", shifted(monday, 9).as_str()),
                ("/plan/weeks/1/week_start", shifted(monday, 7).as_str()),
                ("/plan/weeks_deferred", "0"),
                ("/today", shifted(day, 0).as_str()),
            ]),
            "{zone}: {route}"
        );
        // The tool answers the same weeks, each row stamped as the plan is:
        // whole seconds and `Z`.
        assert_eq!(
            dates_in(&tool),
            table(&[
                ("/plan/created_at", "2026-08-16T23:59:03Z"),
                ("/plan/goal_race/date", shifted(monday, 61).as_str()),
                ("/plan/phases/0/start", shifted(monday, -7).as_str()),
                ("/plan/updated_at", "2026-08-16T23:59:03Z"),
                ("/weeks/0/created_at", "2026-08-17T06:00:00Z"),
                ("/weeks/0/days/0/date", shifted(monday, 0).as_str()),
                ("/weeks/0/days/1/date", shifted(monday, 3).as_str()),
                ("/weeks/0/updated_at", "2026-08-17T06:00:00Z"),
                ("/weeks/0/week_start", shifted(monday, 0).as_str()),
                ("/weeks/1/created_at", "2026-08-18T07:30:59Z"),
                ("/weeks/1/days/0/date", shifted(monday, 9).as_str()),
                ("/weeks/1/updated_at", "2026-08-18T07:30:59Z"),
                ("/weeks/1/week_start", shifted(monday, 7).as_str()),
            ]),
            "{zone}: {tool}"
        );
    }
}

#[tokio::test]
async fn the_activity_list_anchors_today_on_the_athletes_calendar() {
    let resources = common::create_test_server_resources().await.unwrap();
    for zone in ZONES {
        let (today, payload) = loop {
            let today = Today::read();
            let athlete = seed_athlete(&resources, "golden-today-list", Some(zone), "en").await;
            cache_a_ride(&resources, &athlete, &today).await;
            let payload = tool_payload(
                &resources,
                &athlete,
                "get_activities",
                json!({ "provider": UPLOAD, "limit": 10 }),
            )
            .await;
            if today.holds() {
                break (today, payload);
            }
        };
        let day = shifted(today.in_zone(zone), 0);
        let ridden = ride_day(&today, zone);
        let (clock, offset) = ride_clock(zone);
        let start = format!("{}T11:30:00.500250+00:00", shifted(today.utc, -1));

        assert_eq!(
            dates_in(&payload),
            table(&[
                ("/activities/0/start_date", start.as_str()),
                (
                    "/activities/0/start_date_local",
                    format!("{ridden}T{clock}:00.500250{offset}").as_str()
                ),
                (
                    "/activity_list#0",
                    format!("{day} {}", weekday_word(&day, "en")).as_str()
                ),
                (
                    "/activity_list#1",
                    format!("{ridden} {}", weekday_word(&ridden, "en")).as_str()
                ),
                (
                    "/activity_list#2",
                    format!("{ridden} {} {clock}", weekday_word(&ridden, "en")).as_str()
                ),
                ("/retrieval_context/date_range/end", start.as_str()),
                ("/retrieval_context/date_range/start", start.as_str()),
            ]),
            "{zone}: {payload}"
        );
    }
}

/// One ride that starts on a Monday where the athlete lives and on another
/// day in UTC, or the reverse, always in the past: at 11:30 UTC on the Sunday
/// before last week's Monday for Kiritimati, where it is already that Monday,
/// and at 11:30 UTC on last week's Monday west of the line, where it is still
/// the Sunday before. Answers the Monday of the week it belongs to there.
async fn cache_a_ride_on_the_monday_edge(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    today: &Today,
    zone: &str,
) -> String {
    let last_monday = day(&shifted(monday_of(today.utc), -7));
    let (started, week) = if zone == KIRITIMATI {
        (shifted(last_monday, -1), shifted(last_monday, 0))
    } else {
        (shifted(last_monday, 0), shifted(last_monday, -7))
    };
    let ride = ActivityBuilder::new(
        "edge-ride",
        "Ride",
        SportType::Ride,
        instant(&format!("{started}T11:30:00.500250")),
        3_600,
        UPLOAD,
    )
    .distance_meters(30_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, UPLOAD, &[ride])
        .await
        .unwrap();
    week
}

#[tokio::test]
async fn the_weekly_volume_starts_each_week_on_the_athletes_monday() {
    let resources = common::create_test_server_resources().await.unwrap();
    for zone in ZONES {
        let (today, week, body) = loop {
            let today = Today::read();
            let athlete = seed_athlete(&resources, "golden-today-volume", Some(zone), "en").await;
            let week = cache_a_ride_on_the_monday_edge(&resources, &athlete, &today, zone).await;
            let body = home(&resources, &athlete, "/api/me/training-volume").await;
            if today.holds() {
                break (today, week, body);
            }
        };
        let here = today.in_zone(zone);

        // One row a week from the ride's week, on the athlete's calendar, to
        // this one; the ride is counted in the first and in no other.
        let mut expected = vec![("/today".to_owned(), shifted(here, 0))];
        let mut counted = Vec::new();
        let mut monday = day(&week);
        while monday <= monday_of(here) {
            let index = counted.len();
            expected.push((format!("/weeks/{index}/week_start"), shifted(monday, 0)));
            counted.push(u64::from(index == 0));
            monday = day(&shifted(monday, 7));
        }
        expected.sort();
        assert_eq!(dates_in(&body), expected, "{zone}: {body}");
        let rides: Vec<u64> = body["weeks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|week| {
                week["sports"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|sport| sport["activities"].as_u64())
                    .sum()
            })
            .collect();
        assert_eq!(rides, counted, "{zone}: {body}");
    }
}
