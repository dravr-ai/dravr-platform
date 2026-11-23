// ABOUTME: Response date goldens for the training history tools and the training status route
// ABOUTME: Pins each day of the rollup on the athlete's calendar in two zones a day apart, and how from and to are read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `get_training_history`,
//! `compute_training_history` and `/api/me/training-status`
//! (carnet#1620, gate 3).
//!
//! The rollup buckets each workout on the athlete's civil date and writes
//! every date by hand (`%Y-%m-%d`), so the suite seeds 120 daily runs at
//! 12:00:00.500250 on the athlete's own calendar in `Pacific/Kiritimati` and
//! in `Etc/GMT+12`, which is the day before in UTC for the first and the day
//! after for the second, and compares every date of each answer with the days
//! computed from one reading of the clock. The last six runs are
//! long and hard, so the days up to today read as deep fatigue and that list
//! renders too.
//!
//! A window nobody passes defaults to the 90 days up to the athlete's today.
//! The stored history is 120 days deep and a day needs 72 days behind it, so
//! the answer starts 72 days after the oldest run and says so in prose.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::http::StatusCode;
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{
    ActivityBuilder, AnchorProvenance, SportType, ThresholdProvenance, ThresholdStatedOn,
    UserPhysiologicalProfile,
};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    dates_in, day, days_between, get, instant, mcp_call, mcp_payload, mcp_refusal, noon_in,
    seed_athlete, send, shifted, tool_payload, try_tool, with_values, Athlete, Today, KIRITIMATI,
    WEST_OF_THE_DATE_LINE,
};

const ZONES: [&str; 2] = [KIRITIMATI, WEST_OF_THE_DATE_LINE];

/// How many days of runs are stored, the newest the day before the
/// athlete's today.
const STORED_DAYS: i64 = 120;
/// How many deep-fatigue days end on the athlete's today after the six hard
/// days that end yesterday.
const DEEP_FATIGUE_DAYS: i64 = 6;
/// The days of history a day needs behind it before it is answered.
const WARM_UP_DAYS: i64 = 72;

/// An athlete in `zone` with thresholds on file and [`STORED_DAYS`] daily
/// runs at noon on their own calendar ([`noon_in`]): an easy half hour, and
/// four hard hours on each of the last six days.
async fn athlete_with_history(
    resources: &Arc<ServerContext>,
    zone: &str,
    today: &Today,
) -> Athlete {
    let athlete = seed_athlete(resources, "golden-history", Some(zone), "en").await;
    let repos = &resources.common.repos;
    repos
        .user_physiological_profile
        .upsert_user_physiological_profile(
            athlete.tenant,
            athlete.user_id,
            &UserPhysiologicalProfile {
                user_id: athlete.user_id,
                vo2_max: Some(52.0),
                resting_hr: Some(50),
                max_hr: Some(190),
                lactate_threshold_percentage: Some(0.85),
                threshold_hr: None,
                age: Some(34),
                weight: Some(72.0),
                fitness_level: FitnessLevel::Advanced,
                primary_sport: SportType::Run,
                training_experience_years: Some(10),
                ftp_watts: Some(280),
                threshold_pace_sec_per_km: Some(225.0),
                hr_zones: None,
                power_zones: None,
                critical_power_watts: None,
                w_prime_joules: None,
                critical_speed_mps: None,
                d_prime_meters: None,
                threshold_provenance: ThresholdProvenance::default(),
                threshold_stated_on: ThresholdStatedOn::default(),
                vo2_max_method: None,
                vdot: None,
                anchor_provenance: AnchorProvenance::default(),
                transport_policy: TransportPolicy::AnyTransport,
                sex: None,
            },
        )
        .await
        .unwrap();
    let here = today.in_zone(zone);
    let runs: Vec<_> = (1..=STORED_DAYS)
        .map(|back| {
            let (seconds, heart_rate) = if back <= 6 {
                (14_400, 178)
            } else {
                (1_800, 120)
            };
            ActivityBuilder::new(
                format!("run-{back}"),
                format!("run {back} days back"),
                SportType::Run,
                instant(&noon_in(day(&shifted(here, -back)), zone)),
                seconds,
                UPLOAD,
            )
            .distance_meters(6_000.0)
            .average_heart_rate(heart_rate)
            .build()
        })
        .collect();
    repos
        .activity_cache
        .upsert_activities(athlete.user_id, &athlete.tenant, UPLOAD, &runs)
        .await
        .unwrap();
    athlete
}

/// The day of the run seeded `back` days before the athlete's today: each
/// run starts at noon on the athlete's own calendar.
fn run_day(today: &Today, zone: &str, back: i64) -> String {
    shifted(today.in_zone(zone), -back)
}

/// The rows every answer over the default window carries: the window, how
/// far the stored history reaches, and the first day it stands behind,
/// written four times in the note.
fn default_window_rows(today: &Today, zone: &str) -> Vec<(String, String)> {
    let here = today.in_zone(zone);
    let oldest = run_day(today, zone, STORED_DAYS);
    let trusted = shifted(day(&oldest), WARM_UP_DAYS);
    let asked_from = shifted(here, -90);
    vec![
        ("/to".to_owned(), shifted(here, 0)),
        ("/coverage/window/asked_from".to_owned(), asked_from.clone()),
        ("/coverage/window/asked_to".to_owned(), shifted(here, 0)),
        ("/coverage/window/oldest".to_owned(), oldest),
        ("/coverage/trustworthy_from".to_owned(), trusted.clone()),
        // "from {trusted} onward, so days between {asked} and {trusted}
        // have no rows ... only for {trusted} onward".
        ("/coverage/note#0".to_owned(), trusted.clone()),
        ("/coverage/note#1".to_owned(), format!("{asked_from} and")),
        ("/coverage/note#2".to_owned(), format!("{trusted} have")),
        ("/coverage/note#3".to_owned(), trusted),
    ]
}

#[tokio::test]
async fn the_history_over_the_default_window_is_dated_on_the_athletes_calendar() {
    let resources = common::create_test_server_resources().await.unwrap();
    for zone in ZONES {
        let (today, computed, read) = loop {
            let today = Today::read();
            let athlete = athlete_with_history(&resources, zone, &today).await;
            let computed =
                tool_payload(&resources, &athlete, "compute_training_history", json!({})).await;
            let read = tool_payload(&resources, &athlete, "get_training_history", json!({})).await;
            if today.holds() {
                break (today, computed, read);
            }
        };
        let here = today.in_zone(zone);
        let trusted = shifted(day(&run_day(&today, zone, STORED_DAYS)), WARM_UP_DAYS);
        let days = days_between(day(&trusted), here) + 1;

        // The write answers the first day it computed as `from`, and the day
        // asked for beside it.
        let mut expected = default_window_rows(&today, zone);
        expected.extend([
            ("/from".to_owned(), trusted.clone()),
            ("/requested_from".to_owned(), shifted(here, -90)),
            ("/coverage/window/returned".to_owned(), days.to_string()),
            ("/rows_upserted".to_owned(), days.to_string()),
        ]);
        expected.sort();
        assert_eq!(
            with_values(
                dates_in(&computed),
                &computed,
                &["/coverage/window/returned", "/rows_upserted"]
            ),
            expected,
            "{zone}: {computed}"
        );

        // The read answers the window asked for, one row a day from the
        // first day it stands behind, the last day a source delivered, and
        // the run of deep-fatigue days that ends today.
        let mut expected = default_window_rows(&today, zone);
        expected.extend([
            ("/from".to_owned(), shifted(here, -90)),
            ("/coverage/window/returned".to_owned(), days.to_string()),
            (
                "/sources/0/last_activity".to_owned(),
                run_day(&today, zone, 1),
            ),
        ]);
        for index in 0..days {
            expected.push((format!("/days/{index}/date"), shifted(day(&trusted), index)));
        }
        // The six hard days end yesterday, so the run of deep-fatigue days
        // that ends today has a known length, at any hour and in either zone.
        let oldest_first = (0..DEEP_FATIGUE_DAYS).rev().enumerate();
        for (index, back) in oldest_first {
            expected.push((
                format!("/deep_fatigue_mornings/{index}/date"),
                shifted(here, -back),
            ));
        }
        expected.sort();
        assert_eq!(
            with_values(dates_in(&read), &read, &["/coverage/window/returned"]),
            expected,
            "{zone}: {read}"
        );
    }
}

#[tokio::test]
async fn the_training_status_trend_is_dated_on_the_athletes_calendar() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    for zone in ZONES {
        let (today, body) = loop {
            let today = Today::read();
            let athlete = athlete_with_history(&resources, zone, &today).await;
            let (status, body) =
                send(app.clone(), get("/api/me/training-status", &athlete.token)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            if today.holds() {
                break (today, body);
            }
        };
        let here = today.in_zone(zone);

        // Six weeks and today: 43 days, the last the athlete's today.
        let mut expected = vec![("/today".to_owned(), shifted(here, 0))];
        for index in 0..43 {
            expected.push((format!("/trend/{index}/date"), shifted(here, index - 42)));
        }
        expected.sort();
        assert_eq!(dates_in(&body), expected, "{zone}: {body}");
    }
}

#[tokio::test]
async fn from_and_to_are_read_as_calendar_dates_only() {
    let resources = common::create_test_server_resources().await.unwrap();
    let today = Today::read();
    let athlete = athlete_with_history(&resources, KIRITIMATI, &today).await;
    let here = today.in_zone(KIRITIMATI);
    let (from, to) = (shifted(here, -3), shifted(here, -1));
    // The same days with a digit unpadded, where they have one to lose.
    let loose = |date: &str| {
        let parts: Vec<String> = date
            .split('-')
            .map(|part| part.parse::<u32>().unwrap().to_string())
            .collect();
        parts.join("-")
    };

    for tool in ["get_training_history", "compute_training_history"] {
        // Read: echoed padded.
        for (asked_from, asked_to) in [(from.clone(), to.clone()), (loose(&from), loose(&to))] {
            let payload = tool_payload(
                &resources,
                &athlete,
                tool,
                json!({ "from": asked_from, "to": asked_to }),
            )
            .await;
            assert_eq!(
                (payload["from"].as_str(), payload["to"].as_str()),
                (Some(from.as_str()), Some(to.as_str())),
                "{tool} {asked_from}..{asked_to}: {payload}"
            );
        }

        // Refused, each naming the bound and why.
        let refused: [(Value, &str); 7] = [
            (
                json!({ "from": format!("{from}T00:00:00Z"), "to": to }),
                "from must be YYYY-MM-DD: trailing input",
            ),
            (
                json!({ "from": from, "to": format!("{to} 00:00:00") }),
                "to must be YYYY-MM-DD: trailing input",
            ),
            (
                json!({ "from": from.replace('-', ""), "to": to }),
                "from must be YYYY-MM-DD: input contains invalid characters",
            ),
            (
                json!({ "from": "24/08/2026", "to": to }),
                "from must be YYYY-MM-DD: input contains invalid characters",
            ),
            (
                json!({ "from": "2026-02-30", "to": "2026-03-02" }),
                "from must be YYYY-MM-DD: input is out of range",
            ),
            (json!({ "from": to, "to": from }), "from > to"),
            (
                json!({ "from": "2020-01-01", "to": to }),
                "window exceeds the maximum of 365 days",
            ),
        ];
        for (arguments, why) in refused {
            let refusal = try_tool(&resources, &athlete, tool, arguments.clone())
                .await
                .map(|response| format!("answered: {:?} {:?}", response.result, response.error));
            assert_eq!(
                refusal,
                Err(format!("Invalid parameters: {tool}: {why}")),
                "{arguments}"
            );
        }
    }
}

/// The read as an MCP client receives it, through `POST /mcp`: the same
/// dates as the agent's own call in both blocks, and a refused bound's
/// reason as text.
#[tokio::test]
async fn the_history_over_mcp_carries_the_same_dates_and_the_same_refusal() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (today, direct, envelope) = loop {
        let today = Today::read();
        let athlete = athlete_with_history(&resources, KIRITIMATI, &today).await;
        let here = today.in_zone(KIRITIMATI);
        let window = json!({ "from": shifted(here, -2), "to": shifted(here, 0) });
        let direct =
            tool_payload(&resources, &athlete, "get_training_history", window.clone()).await;
        let envelope = mcp_call(&resources, &athlete, "get_training_history", window).await;
        if today.holds() {
            break (today, direct, envelope);
        }
    };
    let here = today.in_zone(KIRITIMATI);
    let payload = mcp_payload(&envelope);

    assert_eq!(dates_in(&payload), dates_in(&direct), "{envelope}");
    let mut expected: Vec<(String, String)> = [
        ("/from", shifted(here, -2)),
        ("/to", shifted(here, 0)),
        ("/coverage/window/asked_from", shifted(here, -2)),
        ("/coverage/window/asked_to", shifted(here, 0)),
        // The read reaches 73 days behind the first day asked for, and the
        // oldest run in it started that day in Kiritimati.
        ("/coverage/window/oldest", shifted(here, -75)),
        ("/days/0/date", shifted(here, -2)),
        ("/days/1/date", shifted(here, -1)),
        ("/days/2/date", shifted(here, 0)),
        ("/deep_fatigue_mornings/0/date", shifted(here, -2)),
        ("/deep_fatigue_mornings/1/date", shifted(here, -1)),
        ("/deep_fatigue_mornings/2/date", shifted(here, 0)),
        ("/sources/0/last_activity", run_day(&today, KIRITIMATI, 1)),
    ]
    .into_iter()
    .map(|(pointer, date)| (pointer.to_owned(), date))
    .collect();
    expected.sort();
    assert_eq!(dates_in(&payload), expected, "{envelope}");

    let athlete = athlete_with_history(&resources, KIRITIMATI, &today).await;
    let refused = mcp_call(
        &resources,
        &athlete,
        "get_training_history",
        json!({ "from": "2026-08-01T00:00:00Z" }),
    )
    .await;
    assert_eq!(
        mcp_refusal(&refused),
        "Tool execution failed: Invalid parameters: get_training_history: from must be \
         YYYY-MM-DD: trailing input",
        "a bound with a time of day is refused over MCP, naming the bound and why"
    );
}
