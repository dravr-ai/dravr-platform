// ABOUTME: Response date goldens for the activity and goal tools, through the tool dispatch the agent uses
// ABOUTME: Pins start_date, start_date_local in Toronto and Kolkata, the list's wall clock, and the goal's stored instant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `get_activities`, `set_goal`, `track_progress`
//! (carnet#1620, gate 3).
//!
//! Activities are written to the cache at instants the suite chooses, under
//! the upload source, which the tools read with no provider connected. Each
//! test calls the tool through `UniversalToolExecutor::execute_tool` and
//! compares every date in the payload with a literal table.
//!
//! Stamped with the wall clock, and so compared by shape only:
//!
//! - `get_activities`: the `[Today]` anchor on the first line of
//!   `activity_list` (`/activity_list#0`), here; its value is compared in
//!   `date_response_today_golden_test`, in two zones a day apart;
//! - `set_goal`: `created_at`, whose shape keeps the clock's fraction.
//!
//! `days_spanned` and `weeks_covered` are numbers computed from the starts,
//! and each list table carries them as rows.
//!
//! `start_date_local` is written in two places. Rows served as summaries are
//! the August workouts, in Toronto and in Kolkata. A list served in full is
//! reached only by rows of the last fourteen days, so that one test dates its
//! rows from today and pins their times of day and offset, in Kolkata.
//!
//! `track_progress` answers no date. It pins the comparison (which
//! activities started after the instant the goal was stored with) and
//! `days_remaining`, the whole days left of the goal's timeframe, on goals
//! stored a chosen number of days before now.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{Activity, ActivityBuilder, SportType};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_detail_store::store_detail;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    call_tool, clock_shape, dates_in, dates_in_stamped, epoch, instant, mcp_call, mcp_payload,
    on_the_clock, seed_athlete, shifted, table, tool_payload, utc_days_before_today, weekday_word,
    with_values, Athlete, Today,
};

const TORONTO: &str = "America/Toronto";
const KOLKATA: &str = "Asia/Kolkata";

/// The two numbers a list computes from its starts.
const SPAN: [&str; 2] = [
    "/retrieval_context/date_range/days_spanned",
    "/retrieval_context/data_sufficiency/weeks_covered",
];

/// Every date in a list payload, and its [`SPAN`].
fn listed(payload: &Value) -> Vec<(String, String)> {
    with_values(
        dates_in_stamped(payload, &["/activity_list#0"]),
        payload,
        &SPAN,
    )
}

/// Three workouts whose starts carry no fraction, milliseconds and
/// microseconds. The ride starts on the 28th in UTC and in Kolkata, and on
/// the evening of the 27th in Toronto.
fn workouts() -> Vec<Activity> {
    vec![
        ActivityBuilder::new(
            "u-ride",
            "Night ride",
            SportType::Ride,
            instant("2026-08-28T02:59:03.500"),
            3_600,
            UPLOAD,
        )
        .distance_meters(30_000.0)
        .build(),
        ActivityBuilder::new(
            "u-swim",
            "Pool",
            SportType::Swim,
            instant("2026-08-29T10:15:30.250125"),
            2_705,
            UPLOAD,
        )
        .distance_meters(2_000.0)
        .build(),
        ActivityBuilder::new(
            "u-run",
            "Morning run",
            SportType::Run,
            instant("2026-08-30T12:00:00"),
            1_800,
            UPLOAD,
        )
        .distance_meters(5_000.0)
        .build(),
    ]
}

async fn cache(resources: &Arc<ServerContext>, athlete: &Athlete, rows: &[Activity]) {
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, UPLOAD, rows)
        .await
        .unwrap();
}

async fn activities(resources: &Arc<ServerContext>, athlete: &Athlete, extra: Value) -> Value {
    let mut parameters = json!({ "provider": UPLOAD, "limit": 10 });
    for (key, value) in extra.as_object().unwrap() {
        parameters[key] = value.clone();
    }
    tool_payload(resources, athlete, "get_activities", parameters).await
}

#[tokio::test]
async fn the_activity_list_in_toronto_states_each_start_twice() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-toronto", Some(TORONTO), "en").await;
        cache(&resources, &athlete, &workouts()).await;

        let payload = activities(&resources, &athlete, json!({})).await;

        assert_eq!(
            listed(&payload),
            table(&[
                ("/activities/0/start_date", "2026-08-30T12:00:00+00:00"),
                (
                    "/activities/0/start_date_local",
                    "2026-08-30T08:00:00-04:00"
                ),
                (
                    "/activities/1/start_date",
                    "2026-08-29T10:15:30.250125+00:00"
                ),
                (
                    "/activities/1/start_date_local",
                    "2026-08-29T06:15:30.250125-04:00"
                ),
                ("/activities/2/start_date", "2026-08-28T02:59:03.500+00:00"),
                (
                    "/activities/2/start_date_local",
                    "2026-08-27T22:59:03.500-04:00"
                ),
                ("/activity_list#0", "dddd-dd-dd"),
                ("/activity_list#1", "2026-08-30 Sun"),
                ("/activity_list#2", "2026-08-30 Sun 08:00"),
                ("/activity_list#3", "2026-08-29 Sat 06:15"),
                ("/activity_list#4", "2026-08-27 Thu 22:59"),
                (
                    "/retrieval_context/date_range/end",
                    "2026-08-30T12:00:00+00:00"
                ),
                (
                    "/retrieval_context/date_range/start",
                    "2026-08-28T02:59:03.500+00:00"
                ),
                // 2 days and 9 hours from the first start to the last.
                ("/retrieval_context/date_range/days_spanned", "2"),
                ("/retrieval_context/data_sufficiency/weeks_covered", "0"),
            ]),
            "{payload}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_activity_list_in_kolkata_states_each_start_twice() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-kolkata", Some(KOLKATA), "fr").await;
        cache(&resources, &athlete, &workouts()).await;

        let payload = activities(&resources, &athlete, json!({})).await;

        assert_eq!(
            listed(&payload),
            table(&[
                ("/activities/0/start_date", "2026-08-30T12:00:00+00:00"),
                (
                    "/activities/0/start_date_local",
                    "2026-08-30T17:30:00+05:30"
                ),
                (
                    "/activities/1/start_date",
                    "2026-08-29T10:15:30.250125+00:00"
                ),
                (
                    "/activities/1/start_date_local",
                    "2026-08-29T15:45:30.250125+05:30"
                ),
                ("/activities/2/start_date", "2026-08-28T02:59:03.500+00:00"),
                (
                    "/activities/2/start_date_local",
                    "2026-08-28T08:29:03.500+05:30"
                ),
                ("/activity_list#0", "dddd-dd-dd"),
                ("/activity_list#1", "2026-08-30 dim"),
                ("/activity_list#2", "2026-08-30 dim 17:30"),
                ("/activity_list#3", "2026-08-29 sam 15:45"),
                ("/activity_list#4", "2026-08-28 ven 08:29"),
                (
                    "/retrieval_context/date_range/end",
                    "2026-08-30T12:00:00+00:00"
                ),
                (
                    "/retrieval_context/date_range/start",
                    "2026-08-28T02:59:03.500+00:00"
                ),
                // 2 days and 9 hours from the first start to the last.
                ("/retrieval_context/date_range/days_spanned", "2"),
                ("/retrieval_context/data_sufficiency/weeks_covered", "0"),
            ]),
            "{payload}"
        );
    })
    .await;
}

/// A list is served in full, from the other place that writes
/// `start_date_local`, when its rows are of the last fourteen days, each has
/// its detail on file, and the read is the second identical one. The days
/// cannot be chosen, so they are yesterday and the day before in UTC; the
/// times of day can, and Kolkata, which never changes its offset, is the
/// same day five and a half hours later.
#[tokio::test]
async fn a_list_served_in_full_in_kolkata_states_each_start_twice() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-detailed", Some(KOLKATA), "fr").await;
        let [yesterday, day_before] = utc_days_before_today([1, 2]);
        let recent = [
            ActivityBuilder::new(
                "u-recent-run",
                "Morning run",
                SportType::Run,
                instant(&format!("{yesterday}T02:59:03.500")),
                1_800,
                UPLOAD,
            )
            .distance_meters(5_000.0)
            .build(),
            ActivityBuilder::new(
                "u-recent-ride",
                "Ride",
                SportType::Ride,
                instant(&format!("{day_before}T12:00:00")),
                3_600,
                UPLOAD,
            )
            .distance_meters(30_000.0)
            .build(),
        ];
        cache(&resources, &athlete, &recent).await;
        let runtime: Arc<dyn ToolRuntime> = resources.clone();
        for workout in &recent {
            let kept = store_detail(
                runtime.repos(),
                runtime.live_activities().as_ref(),
                athlete.tenant,
                athlete.user_id,
                UPLOAD,
                workout,
            )
            .await;
            assert!(kept, "{} is cached", workout.id());
        }

        let first = activities(&resources, &athlete, json!({})).await;
        let payload = activities(&resources, &athlete, json!({})).await;

        assert_eq!(payload["mode"], "detailed", "first {first}, then {payload}");
        // The whole payload: the list's lines and the range are built from the
        // same two days as the rows.
        let (run_day, ride_day) = (
            weekday_word(&yesterday, "fr"),
            weekday_word(&day_before, "fr"),
        );
        assert_eq!(
            listed(&payload),
            table(&[
                (
                    "/activities/0/start_date",
                    format!("{yesterday}T02:59:03.500Z").as_str()
                ),
                (
                    "/activities/0/start_date_local",
                    format!("{yesterday}T08:29:03.500+05:30").as_str()
                ),
                (
                    "/activities/1/start_date",
                    format!("{day_before}T12:00:00Z").as_str()
                ),
                (
                    "/activities/1/start_date_local",
                    format!("{day_before}T17:30:00+05:30").as_str()
                ),
                ("/activity_list#0", "dddd-dd-dd"),
                (
                    "/activity_list#1",
                    format!("{yesterday} {run_day}").as_str()
                ),
                (
                    "/activity_list#2",
                    format!("{yesterday} {run_day} 08:29").as_str()
                ),
                (
                    "/activity_list#3",
                    format!("{day_before} {ride_day} 17:30").as_str()
                ),
                (
                    "/retrieval_context/date_range/end",
                    format!("{yesterday}T02:59:03.500+00:00").as_str()
                ),
                (
                    "/retrieval_context/date_range/start",
                    format!("{day_before}T12:00:00+00:00").as_str()
                ),
                // 14 hours and 59 minutes from one start to the next.
                ("/retrieval_context/date_range/days_spanned", "0"),
                ("/retrieval_context/data_sufficiency/weeks_covered", "0"),
            ]),
            "{payload}"
        );
    })
    .await;
}

#[tokio::test]
async fn an_athlete_with_no_zone_on_file_reads_utc_and_no_local_start() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-no-zone", None, "en").await;
        cache(&resources, &athlete, &workouts()).await;

        let payload = activities(&resources, &athlete, json!({})).await;

        assert_eq!(
            listed(&payload),
            table(&[
                ("/activities/0/start_date", "2026-08-30T12:00:00+00:00"),
                (
                    "/activities/1/start_date",
                    "2026-08-29T10:15:30.250125+00:00"
                ),
                ("/activities/2/start_date", "2026-08-28T02:59:03.500+00:00"),
                ("/activity_list#0", "dddd-dd-dd"),
                ("/activity_list#1", "2026-08-30 Sun"),
                ("/activity_list#2", "2026-08-30 Sun 12:00"),
                ("/activity_list#3", "2026-08-29 Sat 10:15"),
                ("/activity_list#4", "2026-08-28 Fri 02:59"),
                (
                    "/retrieval_context/date_range/end",
                    "2026-08-30T12:00:00+00:00"
                ),
                (
                    "/retrieval_context/date_range/start",
                    "2026-08-28T02:59:03.500+00:00"
                ),
                // 2 days and 9 hours from the first start to the last.
                ("/retrieval_context/date_range/days_spanned", "2"),
                ("/retrieval_context/data_sufficiency/weeks_covered", "0"),
            ]),
            "{payload}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_activity_list_writes_durations_as_hours_minutes_and_seconds() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-durations", Some(TORONTO), "en").await;
    cache(&resources, &athlete, &workouts()).await;

    let payload = activities(&resources, &athlete, json!({})).await;

    let list = payload["activity_list"].as_str().unwrap();
    let ends: Vec<&str> = list
        .lines()
        .filter_map(|line| line.rsplit(" - ").next().filter(|_| line.contains(" km ")))
        .collect();
    assert_eq!(ends, ["0:30:00", "0:45:05", "1:00:00"], "{list}");
}

#[tokio::test]
async fn before_and_after_are_read_as_whole_seconds_since_the_epoch() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-window", Some(TORONTO), "en").await;
    cache(&resources, &athlete, &workouts()).await;
    let started = |payload: &Value| -> Vec<String> {
        payload["activities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["start_date"].as_str().unwrap().to_owned())
            .collect()
    };

    // The swim starts at 10:15:30.250125: a `before` of 10:15:30 leaves it
    // out and one of 10:15:31 keeps it.
    for (before, expected) in [
        (
            epoch("2026-08-29T10:15:30"),
            vec!["2026-08-28T02:59:03.500+00:00"],
        ),
        (
            epoch("2026-08-29T10:15:31"),
            vec![
                "2026-08-29T10:15:30.250125+00:00",
                "2026-08-28T02:59:03.500+00:00",
            ],
        ),
    ] {
        let payload = activities(&resources, &athlete, json!({ "before": before })).await;
        assert_eq!(started(&payload), expected, "before {before}: {payload}");
    }

    // `after` is read the same way: the run starts at 12:00:00, and an
    // `after` of that second or the one before keeps it.
    for (after, expected) in [
        (
            epoch("2026-08-30T12:00:00"),
            vec!["2026-08-30T12:00:00+00:00"],
        ),
        (epoch("2026-08-30T12:00:01"), vec![]),
        (
            epoch("2026-08-29T10:15:30"),
            vec![
                "2026-08-30T12:00:00+00:00",
                "2026-08-29T10:15:30.250125+00:00",
            ],
        ),
        (
            epoch("2026-08-29T10:15:31"),
            vec!["2026-08-30T12:00:00+00:00"],
        ),
    ] {
        let payload = activities(&resources, &athlete, json!({ "after": after })).await;
        assert_eq!(started(&payload), expected, "after {after}: {payload}");
    }

    // A start on the bound's own second is kept by both: the run starts at
    // 12:00:00 and a `before` of 12:00:00 keeps it, one of 11:59:59 does not.
    for (before, expected) in [
        (
            epoch("2026-08-30T12:00:00"),
            vec![
                "2026-08-30T12:00:00+00:00",
                "2026-08-29T10:15:30.250125+00:00",
            ],
        ),
        (
            epoch("2026-08-30T11:59:59"),
            vec!["2026-08-29T10:15:30.250125+00:00"],
        ),
    ] {
        let payload = activities(
            &resources,
            &athlete,
            json!({ "after": epoch("2026-08-28T02:59:04"), "before": before }),
        )
        .await;
        assert_eq!(started(&payload), expected, "before {before}: {payload}");
    }

    // An instant written as text is not read: the window stays open.
    for bound in ["before", "after"] {
        let payload = activities(
            &resources,
            &athlete,
            json!({ bound: "2026-08-29T10:15:30Z" }),
        )
        .await;
        assert_eq!(started(&payload).len(), 3, "{bound}: {payload}");
    }
}

/// A distance goal stored with `created_at`, read back through
/// `track_progress`: the kilometres it counts say which workouts started
/// after that instant.
async fn kilometres_since(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    created_at: &str,
) -> f64 {
    let goal_id = resources
        .common
        .repos
        .profiles
        .create_goal(
            athlete.user_id,
            json!({
                "goal_type": "distance",
                "target_value": 100.0,
                "timeframe": "year",
                "title": "A hundred",
                "sport": null,
                "created_at": created_at,
            }),
        )
        .await
        .unwrap();
    let payload = tool_payload(
        resources,
        athlete,
        "track_progress",
        json!({ "goal_id": goal_id, "provider": UPLOAD }),
    )
    .await;
    assert_eq!(dates_in(&payload), table(&[]), "{payload}");
    payload["summary"]["total_distance_km"].as_f64().unwrap()
}

/// Three runs far enough apart to stay three sessions, of 1, 2 and 4 km:
/// the sum names the ones counted. The middle one starts at 02:59:03.500 UTC
/// on the 28th.
fn runs_around_the_stored_instant() -> Vec<Activity> {
    [
        ("early", "2026-08-20T06:00:00", 1_000.0),
        ("at", "2026-08-28T02:59:03.500", 2_000.0),
        ("late", "2026-08-30T12:00:00", 4_000.0),
    ]
    .into_iter()
    .map(|(id, start, metres)| {
        ActivityBuilder::new(id, id, SportType::Run, instant(start), 600, UPLOAD)
            .distance_meters(metres)
            .build()
    })
    .collect()
}

#[tokio::test]
async fn a_stored_goal_counts_the_workouts_started_after_its_instant_in_any_offset() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources, "golden-goal", Some(KOLKATA), "en").await;
    cache(&resources, &athlete, &runs_around_the_stored_instant()).await;

    // (the goal's stored created_at, kilometres counted). 4 is the last run
    // alone, 6 the last two, and 7 every run, which is what a stamp that is
    // not read as an instant counts.
    let stored = [
        // The middle run's own instant, in Kolkata, Toronto and UTC, with `Z`
        // and `+00:00`: a run starting at the instant itself is not after it.
        ("2026-08-28T08:29:03.500+05:30", 4.0),
        ("2026-08-27T22:59:03.500-04:00", 4.0),
        ("2026-08-28T02:59:03.500Z", 4.0),
        ("2026-08-28T02:59:03.500+00:00", 4.0),
        // One millisecond either side, and the whole second.
        ("2026-08-28T08:29:03.499+05:30", 6.0),
        ("2026-08-28T08:29:03.501+05:30", 4.0),
        ("2026-08-28T08:29:03+05:30", 6.0),
        // A space for the `T`.
        ("2026-08-28 08:29:03.500+05:30", 4.0),
        // Not instants: no zone, a date alone, an offset without its colon.
        ("2026-08-28T08:29:03.500", 7.0),
        ("2026-08-28", 7.0),
        ("2026-08-28T08:29:03.500+0530", 7.0),
    ];
    let mut counted = Vec::new();
    for (created_at, _) in stored {
        counted.push((
            created_at,
            kilometres_since(&resources, &athlete, created_at).await,
        ));
    }
    assert_eq!(
        counted, stored,
        "the kilometres counted after each stored created_at"
    );
}

#[tokio::test]
async fn a_new_goal_is_stamped_with_an_offset_and_stored_as_answered() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-set-goal", Some(TORONTO), "en").await;

        let payload = tool_payload(
            &resources,
            &athlete,
            "set_goal",
            json!({
                "goal_type": "distance",
                "target_value": 100.0,
                "timeframe": "monthly",
                "title": "A hundred",
            }),
        )
        .await;

        // Stamped with now: UTC written as an offset, never `Z`, whatever the
        // athlete's zone.
        assert_eq!(
            dates_in_stamped(&payload, &["/created_at"]),
            table(&[("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd")]),
            "{payload}"
        );
        let answered = payload["created_at"].as_str().unwrap();
        assert!(answered.ends_with("+00:00"), "{answered}");

        // The stored goal carries the same text, which `track_progress` reads.
        let goals = resources
            .common
            .repos
            .profiles
            .get_goals(athlete.user_id)
            .await
            .unwrap();
        let stored: Vec<&str> = goals
            .iter()
            .filter_map(|goal| goal["created_at"].as_str())
            .collect();
        assert_eq!(stored, [answered], "{goals:?}");
        assert_eq!(
            clock_shape(answered),
            "dddd-dd-ddTdd:dd:dd.f+dd:dd",
            "the stored goal keeps the clock's fraction and offset: {answered}"
        );

        // A goal set now has no workout after it.
        cache(&resources, &athlete, &workouts()).await;
        let goal_id = payload["goal_id"].as_str().unwrap();
        let progress = call_tool(
            &resources,
            &athlete,
            "track_progress",
            json!({ "goal_id": goal_id, "provider": UPLOAD }),
        )
        .await;
        assert!(progress.success, "{:?}", progress.error);
        assert_eq!(
            progress.result.unwrap()["summary"]["total_activities"],
            0,
            "the workouts of August are before a goal set today"
        );
    })
    .await;
}

/// `days_remaining` is the goal's timeframe less the whole days since it was
/// stored. The goals are stored a chosen time before one reading of the
/// clock, so the count is known whatever the hour.
#[tokio::test]
async fn days_remaining_counts_whole_days_since_the_stored_instant() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (today, stored, counted) = loop {
        let today = Today::read();
        let (ten_ago, eleven_ago) = (shifted(today.utc, -10), shifted(today.utc, -11));
        // (stored created_at, timeframe, days remaining). Midnight UTC ten
        // days ago is ten whole days ago at any hour of today; the last
        // instant of the day before is ten days ago too, and the same
        // instant in Kolkata's offset reads the same.
        let stored = vec![
            (format!("{ten_ago}T00:00:00Z"), "year", 355),
            (format!("{ten_ago}T05:30:00+05:30"), "year", 355),
            (format!("{eleven_ago}T23:59:59.999Z"), "year", 355),
            (format!("{eleven_ago}T00:00:00Z"), "year", 354),
            (format!("{ten_ago}T00:00:00Z"), "month", 20),
            (format!("{ten_ago}T00:00:00Z"), "week", 0),
            // Not an instant: the default timeframe, whole.
            (ten_ago.clone(), "year", 30),
        ];
        // A fresh athlete each time round, so a midnight during one round
        // leaves nothing behind for the next.
        let athlete = seed_athlete(&resources, "golden-goal-days", Some(KOLKATA), "en").await;
        cache(&resources, &athlete, &workouts()).await;
        let mut counted = Vec::new();
        for (created_at, timeframe, _) in &stored {
            let goal_id = resources
                .common
                .repos
                .profiles
                .create_goal(
                    athlete.user_id,
                    json!({
                        "goal_type": "distance",
                        "target_value": 100.0,
                        "timeframe": timeframe,
                        "title": "A hundred",
                        "sport": null,
                        "created_at": created_at,
                    }),
                )
                .await
                .unwrap();
            let payload = tool_payload(
                &resources,
                &athlete,
                "track_progress",
                json!({ "goal_id": goal_id, "provider": UPLOAD }),
            )
            .await;
            counted.push((
                created_at.clone(),
                *timeframe,
                payload["days_remaining"].as_i64().unwrap(),
            ));
        }
        if today.holds() {
            break (today, stored, counted);
        }
    };
    assert_eq!(counted, stored, "today is {}", today.utc);
}

/// The same list as an MCP client receives it, through `POST /mcp`: the
/// envelope's text block and its structured content carry the same dates as
/// the agent's own call.
#[tokio::test]
async fn the_activity_list_over_mcp_carries_the_same_dates_in_both_blocks() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = seed_athlete(&resources, "golden-mcp-list", Some(TORONTO), "en").await;
        cache(&resources, &athlete, &workouts()).await;

        let envelope = mcp_call(
            &resources,
            &athlete,
            "get_activities",
            json!({ "provider": UPLOAD, "limit": 10 }),
        )
        .await;
        let payload = mcp_payload(&envelope);

        assert_eq!(
            listed(&payload),
            table(&[
                ("/activities/0/start_date", "2026-08-30T12:00:00+00:00"),
                (
                    "/activities/0/start_date_local",
                    "2026-08-30T08:00:00-04:00"
                ),
                (
                    "/activities/1/start_date",
                    "2026-08-29T10:15:30.250125+00:00"
                ),
                (
                    "/activities/1/start_date_local",
                    "2026-08-29T06:15:30.250125-04:00"
                ),
                ("/activities/2/start_date", "2026-08-28T02:59:03.500+00:00"),
                (
                    "/activities/2/start_date_local",
                    "2026-08-27T22:59:03.500-04:00"
                ),
                ("/activity_list#0", "dddd-dd-dd"),
                ("/activity_list#1", "2026-08-30 Sun"),
                ("/activity_list#2", "2026-08-30 Sun 08:00"),
                ("/activity_list#3", "2026-08-29 Sat 06:15"),
                ("/activity_list#4", "2026-08-27 Thu 22:59"),
                (
                    "/retrieval_context/date_range/end",
                    "2026-08-30T12:00:00+00:00"
                ),
                (
                    "/retrieval_context/date_range/start",
                    "2026-08-28T02:59:03.500+00:00"
                ),
                ("/retrieval_context/date_range/days_spanned", "2"),
                ("/retrieval_context/data_sufficiency/weeks_covered", "0"),
            ]),
            "{envelope}"
        );
    })
    .await;
}
