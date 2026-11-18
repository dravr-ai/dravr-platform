// ABOUTME: Response date goldens for the tools answered from stored Strava details: segment history and one activity's analysis
// ABOUTME: Pins each effort's instant, the year it is counted in on the athlete's calendar, and the last-synced caveat
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `get_segment_history`, and the caveat on an
//! activity served from its stored copy (`get_activity_intelligence`,
//! `analyze_activity`) (carnet#1620, gate 3).
//!
//! Three rides are stored with their details, each carrying one effort on
//! the same segment: two either side of the new year in UTC, one in August.
//! A sport asked is answered from the stored details, with no listing read
//! from Strava, so the tool runs on the offline server. The summary counts
//! efforts by the year each started in **where the athlete lives**, written
//! by hand (`%Y`): the two around the new year are both 2026 in
//! `Pacific/Kiritimati` and both 2025 in `Etc/GMT+12`.

#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_core::constants::oauth::providers::STRAVA;
use pierre_core::constants::provider_capture::current_capture_version;
use pierre_core::models::{ActivityBuilder, SegmentEffort, SportType};
use pierre_database::repositories::BackfillCoverage;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_detail_store::store_detail;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, instant, mcp_call, mcp_payload, on_the_clock, seed_athlete, table,
    tool_payload, try_tool, with_values, Athlete, KIRITIMATI, WEST_OF_THE_DATE_LINE,
};
use crate::helpers::strava_grant::hold_strava_grant;

const SEGMENT: &str = "611413";

/// The starts of the three rides; each effort begins ten minutes in.
const RIDES: [(&str, &str, &str); 3] = [
    ("1001", "2025-12-31T19:50:00", "2025-12-31T20:00:00"),
    ("1002", "2026-01-01T04:50:00.500", "2026-01-01T05:00:00.500"),
    ("1003", "2026-08-28T02:49:03", "2026-08-28T02:59:03"),
];

async fn athlete_with_efforts(resources: &Arc<ServerContext>, zone: &str) -> Athlete {
    let athlete = seed_athlete(resources, "golden-segments", Some(zone), "en").await;
    hold_strava_grant(resources, athlete.user_id, athlete.tenant).await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let rides: Vec<_> = RIDES
        .iter()
        .map(|(id, started, effort)| {
            ActivityBuilder::new(
                *id,
                "Morning ride",
                SportType::Ride,
                instant(started),
                3_600,
                STRAVA,
            )
            .distance_meters(30_000.0)
            .segment_efforts(vec![SegmentEffort {
                id: format!("{id}-effort"),
                segment_id: Some(SEGMENT.to_owned()),
                name: "La Bete".to_owned(),
                elapsed_time: 412,
                moving_time: Some(412),
                start_date: instant(effort),
                distance: 1_820.5,
                average_heart_rate: Some(161),
                max_heart_rate: None,
                average_cadence: None,
                average_watts: Some(287),
                kom_rank: None,
                pr_rank: Some(1),
                climb_category: None,
                average_grade: None,
                elevation_gain: None,
            }])
            .build()
        })
        .collect();
    runtime
        .live_activities()
        .ingest(
            athlete.user_id,
            athlete.tenant,
            STRAVA,
            &rides,
            instant("2026-08-28T11:00:00.250125"),
        )
        .await
        .unwrap();
    for ride in &rides {
        assert!(
            store_detail(
                runtime.repos(),
                runtime.live_activities().as_ref(),
                athlete.tenant,
                athlete.user_id,
                STRAVA,
                ride,
            )
            .await,
            "the ride's detail is stored"
        );
    }
    resources
        .common
        .repos
        .activity_cache
        .upsert_backfill_coverage(
            athlete.user_id,
            &athlete.tenant,
            STRAVA,
            BackfillCoverage {
                oldest_reached_ts: 0,
                hit_feed_end: true,
                capture_version: current_capture_version(STRAVA),
            },
        )
        .await
        .unwrap();
    athlete
}

fn asked(from: &str, to: &str) -> Value {
    json!({ "segment_id": SEGMENT, "sport_type": "Ride", "from": from, "to": to })
}

#[tokio::test]
async fn efforts_are_counted_in_the_year_they_started_in_where_the_athlete_lives() {
    let resources = common::create_test_server_resources().await.unwrap();
    // (zone, the efforts counted in 2025 and in 2026).
    for (zone, by_year) in [
        (KIRITIMATI, r#"{"2026":3}"#),
        (WEST_OF_THE_DATE_LINE, r#"{"2025":2,"2026":1}"#),
    ] {
        let athlete = athlete_with_efforts(&resources, zone).await;

        let payload = tool_payload(
            &resources,
            &athlete,
            "get_segment_history",
            asked("2025-12-01", "2026-08-31"),
        )
        .await;

        assert_eq!(payload["answered_by"], "stored_details", "{payload}");
        // Each effort's own instant in UTC with an offset, newest first; the
        // window as asked, twice more in the note; and the count by year.
        assert_eq!(
            with_values(dates_in(&payload), &payload, &["/summary/0/by_year"]),
            table(&[
                ("/coverage/note#0", "2025-12-01 to"),
                ("/coverage/note#1", "2026-08-31 have"),
                ("/coverage/window/asked_from", "2025-12-01"),
                ("/coverage/window/asked_to", "2026-08-31"),
                ("/efforts/0/start_date", "2026-08-28T02:59:03+00:00"),
                ("/efforts/1/start_date", "2026-01-01T05:00:00.500+00:00"),
                ("/efforts/2/start_date", "2025-12-31T20:00:00+00:00"),
                ("/summary/0/best_start_date", "2026-08-28T02:59:03+00:00"),
                ("/summary/0/by_year", by_year),
            ]),
            "{zone}: {payload}"
        );
    }
}

/// The efforts a window holds, by their activity, newest first.
async fn held(resources: &Arc<ServerContext>, athlete: &Athlete, from: &str, to: &str) -> String {
    let payload = tool_payload(resources, athlete, "get_segment_history", asked(from, to)).await;
    let ids: Vec<&str> = payload["efforts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|effort| effort["activity_id"].as_str().unwrap())
        .collect();
    ids.join(" ")
}

#[tokio::test]
async fn from_and_to_bound_the_window_on_the_athletes_calendar() {
    let resources = common::create_test_server_resources().await.unwrap();

    // The window is whole days where the athlete lives. In Kiritimati the
    // ride of 19:50 UTC on the 31st started on 1 January; west of the line
    // the ride of 04:50 UTC on the 1st started on 31 December.
    let ahead = athlete_with_efforts(&resources, KIRITIMATI).await;
    let behind = athlete_with_efforts(&resources, WEST_OF_THE_DATE_LINE).await;
    let windows = [
        ("2025-12-31", "2025-12-31", "", "1002 1001"),
        ("2026-01-01", "2026-01-01", "1002 1001", ""),
        ("2026-8-28", "2026-8-28", "1003", ""),
        ("2026-08-27", "2026-08-27", "", "1003"),
    ];
    let mut answered = Vec::new();
    for (from, to, _, _) in windows {
        answered.push((
            from,
            to,
            held(&resources, &ahead, from, to).await,
            held(&resources, &behind, from, to).await,
        ));
    }
    let expected: Vec<(&str, &str, String, String)> = windows
        .iter()
        .map(|(from, to, ahead, behind)| (*from, *to, (*ahead).to_owned(), (*behind).to_owned()))
        .collect();
    assert_eq!(
        answered, expected,
        "(from, to, held in Kiritimati, held west of the line) per window"
    );

    let athlete = ahead;

    let refused = try_tool(
        &resources,
        &athlete,
        "get_segment_history",
        asked("2026-08-28T00:00:00Z", "2026-08-31"),
    )
    .await
    .map(|response| format!("{:?}", response.result));
    assert_eq!(
        refused,
        Err(
            "Invalid parameters: get_segment_history: from must be YYYY-MM-DD: trailing input"
                .to_owned()
        ),
        "a bound with a time of day is refused"
    );
    let backwards = try_tool(
        &resources,
        &athlete,
        "get_segment_history",
        asked("2026-08-31", "2026-08-28"),
    )
    .await
    .map(|response| format!("{:?}", response.result));
    assert_eq!(
        backwards,
        Err("Invalid parameters: get_segment_history: from must not be after to".to_owned()),
        "a window that ends before it starts is refused"
    );
}

/// The offline server reaches no Strava, so a live read of one activity
/// fails and its stored copy answers under a caveat that says when Strava
/// was last read: the sync instant with an offset, and the same instant by
/// hand, cut at the minute, in the note.
#[tokio::test]
async fn a_stored_activity_served_for_an_unread_provider_says_when_it_was_synced() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = athlete_with_efforts(&resources, KIRITIMATI).await;

        for tool in ["get_activity_intelligence", "analyze_activity"] {
            let payload = tool_payload(
                &resources,
                &athlete,
                tool,
                json!({ "activity_id": "1003", "provider": STRAVA }),
            )
            .await;

            // The rides were ingested at 11:00:00.250125 UTC; the athlete's zone
            // does not move the minute the note states.
            assert_eq!(
                dates_in_stamped(&payload, &["/timestamp"]),
                table(&[
                    (
                        "/provider_unavailable/last_synced_at",
                        "2026-08-28T11:00:00.250125+00:00"
                    ),
                    ("/provider_unavailable/note#0", "2026-08-28 11:00 UTC"),
                    ("/timestamp", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ]),
                "{tool}: {payload}"
            );
        }
    })
    .await;
}

/// The same ask as an MCP client makes it, through `POST /mcp`. Strava's
/// terms keep its efforts off that wire, so the client gets the window it
/// asked for, twice more in the note, and a count of what it was not given:
/// no effort, no instant of one.
#[tokio::test]
async fn a_segments_history_over_mcp_answers_the_window_and_withholds_the_efforts() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_with_efforts(&resources, KIRITIMATI).await;

    let envelope = mcp_call(
        &resources,
        &athlete,
        "get_segment_history",
        asked("2025-12-01", "2026-08-31"),
    )
    .await;
    let payload = mcp_payload(&envelope);

    assert_eq!(
        dates_in(&payload),
        table(&[
            ("/coverage/note#0", "2025-12-01 to"),
            ("/coverage/note#1", "2026-08-31 have"),
            ("/coverage/window/asked_from", "2025-12-01"),
            ("/coverage/window/asked_to", "2026-08-31"),
        ]),
        "{envelope}"
    );
    assert_eq!(
        payload["_unavailable_over_this_interface"]["items_unavailable"], 3,
        "{envelope}"
    );
}
