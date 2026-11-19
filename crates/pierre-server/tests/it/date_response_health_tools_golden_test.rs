// ABOUTME: Response date goldens for the stored sleep, recovery and health tools, through the tool dispatch the agent uses
// ABOUTME: Pins the rows' instants and days, the echoed range, the last-sync stamp, and how `start` and `end` are read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `get_sleep_sessions`, `get_recovery_metrics`,
//! `get_health_snapshots` (carnet#1620, gate 3).
//!
//! One night, one recovery day and one body reading are stored under a WHOOP
//! source at instants the suite chooses, with the sync cursor's own stamp.
//! Each tool is called through `UniversalToolExecutor::execute_tool` and every
//! date in its payload is compared with a literal table.
//!
//! A recovery day, a body reading and a day's activity totals answer no
//! `recorded_at` (carnet#1629): the store keeps no reading time for a day,
//! only the row's first write (`created_at`). The row is moved to a chosen
//! instant and the field's absence pinned, so that write time is never
//! answered as a reading time.
//!
//! The three tools read `start` and `end` by hand (`parse_date_range`): text
//! that is not an RFC 3339 instant is not refused, it is ignored, and the
//! window falls back to the wall clock. Those rows are compared by shape,
//! which keeps the clock's fraction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::models::{
    ConnectionType, DataSource, DeviceType, StoredDailyActivity, StoredHealthMetrics,
    StoredRecoveryMetrics, StoredSleepSession,
};
use pierre_database::repositories::{SyncCursorRow, WellnessWriteMode};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, day, instant, mcp_call, mcp_payload, on_the_clock, restamp,
    seed_athlete, table, tool_payload, try_tool, Athlete,
};

const TOOLS: [&str; 3] = [
    "get_sleep_sessions",
    "get_recovery_metrics",
    "get_health_snapshots",
];

/// August 2026, as the agent asks for it.
fn august() -> Value {
    json!({ "start": "2026-08-01T00:00:00Z", "end": "2026-08-31T00:00:00Z" })
}

/// An athlete in Toronto with WHOOP connected and its notice accepted, a
/// night from 02:59:03.500 to 10:14:00 UTC on the 28th, that day's recovery,
/// body reading and activity totals, and each kind last synced at
/// 11:00:00.250125.
async fn athlete_with_a_stored_day(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let athlete = seed_athlete(resources, label, Some("America/Toronto"), "en").await;
    let repos = &resources.common.repos;
    let user = athlete.user_id.to_string();
    repos
        .provider_connections
        .register_connection(
            athlete.user_id,
            athlete.tenant,
            "whoop",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    repos
        .users
        .record_provider_terms(
            athlete.user_id,
            "whoop",
            provider_terms_version("whoop").expect("WHOOP carries a notice"),
        )
        .await
        .unwrap();
    let source = repos
        .data_sources
        .upsert_data_source(
            &athlete.tenant,
            &DataSource {
                id: String::new(),
                user_id: user.clone(),
                provider: "whoop".to_owned(),
                device_model: None,
                software_version: None,
                source: None,
                device_type: DeviceType::Unknown,
                original_source_name: None,
            },
        )
        .await
        .unwrap();
    let synced_at = instant("2026-08-28T11:00:00.250125");

    repos
        .sleep
        .upsert_sleep_session(
            &athlete.tenant,
            &StoredSleepSession {
                id: "whoop-night".to_owned(),
                user_id: user.clone(),
                data_source_id: source.clone(),
                is_nap: false,
                start_datetime: instant("2026-08-28T02:59:03.500250"),
                end_datetime: instant("2026-08-28T10:14:00"),
                total_sleep_seconds: Some(25_000),
                deep_sleep_seconds: None,
                light_sleep_seconds: None,
                rem_sleep_seconds: None,
                awake_seconds: None,
                sleep_efficiency: None,
                avg_heart_rate: None,
                min_heart_rate: None,
                avg_hrv: None,
                sleep_score: None,
                avg_spo2: None,
                stages: Vec::new(),
                source_name: "whoop".to_owned(),
            },
            synced_at,
        )
        .await
        .unwrap();
    repos
        .recovery
        .upsert_recovery_metrics(
            &athlete.tenant,
            &StoredRecoveryMetrics {
                id: "whoop-day".to_owned(),
                user_id: user.clone(),
                data_source_id: source.clone(),
                date: day("2026-08-28"),
                recovery_score: None,
                readiness_score: None,
                hrv_ms: Some(58.4),
                hrv_rmssd: None,
                resting_heart_rate: Some(48),
                stress_score: None,
                body_battery: None,
                spo2: None,
                respiratory_rate: None,
                skin_temp_deviation: None,
                daily_strain: None,
                athlete_note: None,
                source_name: "whoop".to_owned(),
                recorded_at: instant("2026-08-28T10:30:00.125"),
            },
            WellnessWriteMode::FullDay,
            synced_at,
        )
        .await
        .unwrap();
    repos
        .health_snapshots
        .upsert_health_snapshot(
            &athlete.tenant,
            &StoredHealthMetrics {
                id: String::new(),
                user_id: user.clone(),
                data_source_id: source.clone(),
                date: day("2026-08-28"),
                weight_kg: Some(68.2),
                body_fat_pct: None,
                muscle_mass_kg: None,
                bmi: None,
                bone_mass_kg: None,
                water_pct: None,
                systolic_bp: None,
                diastolic_bp: None,
                blood_glucose: None,
                vo2_max: None,
                source_name: "whoop".to_owned(),
                recorded_at: instant("2026-08-28T06:45:10"),
            },
            WellnessWriteMode::FullDay,
            synced_at,
        )
        .await
        .unwrap();
    repos
        .daily_activity
        .upsert_daily_activity(
            &athlete.tenant,
            &StoredDailyActivity {
                id: String::new(),
                user_id: user.clone(),
                data_source_id: source,
                date: day("2026-08-28"),
                steps: Some(11_284),
                active_kcal: None,
                total_kcal: None,
                intensity_minutes: None,
                source_name: "whoop".to_owned(),
                recorded_at: instant("2026-08-28T23:59:00"),
            },
            WellnessWriteMode::FullDay,
            synced_at,
        )
        .await
        .unwrap();
    for data_type in ["sleep", "recovery", "health"] {
        repos
            .sync_cursors
            .upsert_sync_cursor(&SyncCursorRow {
                id: format!("cursor-{data_type}-{user}"),
                user_id: user.clone(),
                tenant_id: athlete.tenant.to_string(),
                provider: "whoop".to_owned(),
                data_type: data_type.to_owned(),
                cursor_value: None,
                last_sync_at: Some(synced_at),
                last_sync_status: "success".to_owned(),
                records_synced: 1,
                error_message: None,
                retry_count: 0,
                next_retry_at: None,
            })
            .await
            .unwrap();
    }
    athlete
}

#[tokio::test]
async fn a_stored_night_answers_its_instants_the_range_and_the_last_sync() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_stored_day(&resources, "golden-sleep").await;

    let payload = tool_payload(&resources, &athlete, "get_sleep_sessions", august()).await;

    assert_eq!(payload["count"], 1, "{payload}");
    assert_eq!(
        dates_in(&payload),
        table(&[
            ("/last_synced_at/whoop", "2026-08-28T11:00:00.250125+00:00"),
            ("/range/end", "2026-08-31T00:00:00+00:00"),
            ("/range/start", "2026-08-01T00:00:00+00:00"),
            ("/sessions/0/end_datetime", "2026-08-28T10:14:00Z"),
            ("/sessions/0/start_datetime", "2026-08-28T02:59:03.500250Z"),
        ]),
        "{payload}"
    );
}

#[tokio::test]
async fn a_stored_recovery_day_answers_its_day_its_reading_and_the_last_sync() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_stored_day(&resources, "golden-recovery").await;

    // The row's write time is moved off the wall clock: were it answered as
    // `recorded_at`, it would show as this instant.
    restamp(
        &resources.agent.database,
        "UPDATE recovery_metrics SET created_at = $1 WHERE user_id = $2 AND CAST(tenant_id AS TEXT) = $3",
        instant("2026-08-28T10:31:07.125375"),
        &athlete.user_id.to_string(),
        Some(athlete.tenant),
    )
    .await;

    let payload = tool_payload(&resources, &athlete, "get_recovery_metrics", august()).await;

    assert_eq!(payload["count"], 1, "{payload}");
    assert_eq!(
        dates_in(&payload),
        table(&[
            ("/last_synced_at/whoop", "2026-08-28T11:00:00.250125+00:00"),
            ("/metrics/0/date", "2026-08-28"),
            ("/range/end", "2026-08-31T00:00:00+00:00"),
            ("/range/start", "2026-08-01T00:00:00+00:00"),
        ]),
        "{payload}"
    );
    assert_eq!(
        payload["metrics"][0].get("recorded_at"),
        None,
        "a recovery day answers no reading time: {payload}"
    );
}

#[tokio::test]
async fn a_stored_body_reading_answers_its_day_its_reading_and_the_last_sync() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_stored_day(&resources, "golden-health").await;

    // As for a recovery day: the write time is moved, and never answered.
    restamp(
        &resources.agent.database,
        "UPDATE health_snapshots SET created_at = $1 WHERE user_id = $2 AND CAST(tenant_id AS TEXT) = $3",
        instant("2026-08-28T06:46:00"),
        &athlete.user_id.to_string(),
        Some(athlete.tenant),
    )
    .await;

    let payload = tool_payload(&resources, &athlete, "get_health_snapshots", august()).await;

    assert_eq!(payload["count"], 1, "{payload}");
    assert_eq!(
        dates_in(&payload),
        table(&[
            ("/daily_activity/0/date", "2026-08-28"),
            ("/last_synced_at/whoop", "2026-08-28T11:00:00.250125+00:00"),
            ("/range/end", "2026-08-31T00:00:00+00:00"),
            ("/range/start", "2026-08-01T00:00:00+00:00"),
            ("/snapshots/0/date", "2026-08-28"),
        ]),
        "{payload}"
    );
    assert_eq!(
        (
            payload["snapshots"][0].get("recorded_at"),
            payload["daily_activity"][0].get("recorded_at"),
        ),
        (None, None),
        "a body reading and a day's totals answer no reading time: {payload}"
    );
}

/// The window each tool echoes for `start` and `end` as the agent wrote them.
async fn echoed(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    arguments: &Value,
    stamped_now: &[&str],
) -> Vec<Vec<(String, String)>> {
    let mut ranges = Vec::new();
    for tool in TOOLS {
        let payload = tool_payload(resources, athlete, tool, arguments.clone()).await;
        ranges.push(dates_in_stamped(&payload["range"], stamped_now));
    }
    ranges
}

#[tokio::test]
async fn start_and_end_are_read_as_instants_and_echoed_in_utc() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = athlete_with_a_stored_day(&resources, "golden-range").await;

        // (start, end, the range echoed).
        let read = [
            // `Z`, an offset, a fraction: all echoed in UTC with `+00:00`.
            (
                "2026-08-01T00:00:00Z",
                "2026-08-31T05:30:00+05:30",
                ("2026-08-01T00:00:00+00:00", "2026-08-31T00:00:00+00:00"),
            ),
            (
                "2026-07-31T20:00:00.250-04:00",
                "2026-08-31T12:00:00.5Z",
                (
                    "2026-08-01T00:00:00.250+00:00",
                    "2026-08-31T12:00:00.500+00:00",
                ),
            ),
            // A space for the `T`.
            (
                "2026-08-01 00:00:00Z",
                "2026-08-31 12:00:00+00:00",
                ("2026-08-01T00:00:00+00:00", "2026-08-31T12:00:00+00:00"),
            ),
            // A `start` that is not an instant is ignored: the window opens
            // thirty days before `end`, not at the day that was written.
            (
                "2026-08-15",
                "2026-08-31T12:00:00Z",
                ("2026-08-01T12:00:00+00:00", "2026-08-31T12:00:00+00:00"),
            ),
            (
                "2026-08-15T00:00:00",
                "2026-08-31T12:00:00Z",
                ("2026-08-01T12:00:00+00:00", "2026-08-31T12:00:00+00:00"),
            ),
            // A longer window is clipped to the 366 days before `end`.
            (
                "2024-01-01T00:00:00Z",
                "2026-08-31T00:00:00Z",
                ("2025-08-30T00:00:00+00:00", "2026-08-31T00:00:00+00:00"),
            ),
        ];
        for (start, end, (echoed_start, echoed_end)) in read {
            let expected = table(&[("/end", echoed_end), ("/start", echoed_start)]);
            assert_eq!(
                echoed(
                    &resources,
                    &athlete,
                    &json!({ "start": start, "end": end }),
                    &[]
                )
                .await,
                vec![expected; TOOLS.len()],
                "start {start}, end {end}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn an_end_that_is_not_an_instant_is_ignored_for_the_wall_clock() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let athlete = athlete_with_a_stored_day(&resources, "golden-range-now").await;
        let now_to_now = table(&[
            ("/end", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ("/start", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
        ]);

        // A date alone, an instant with no zone, no arguments at all: the window
        // is the thirty days up to now, stamped with now.
        for arguments in [
            json!({ "start": "2026-08-01", "end": "2026-08-31" }),
            json!({ "end": "2026-08-31T00:00:00" }),
            json!({}),
        ] {
            assert_eq!(
                echoed(&resources, &athlete, &arguments, &["/start", "/end"]).await,
                vec![now_to_now.clone(); TOOLS.len()],
                "{arguments}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn a_start_after_the_end_is_refused_naming_both_instants() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_stored_day(&resources, "golden-range-refused").await;

    for tool in TOOLS {
        let refusal = try_tool(
            &resources,
            &athlete,
            tool,
            json!({ "start": "2026-09-01T05:30:00.250+05:30", "end": "2026-08-31T00:00:00Z" }),
        )
        .await
        .expect_err("a backwards window is refused");
        assert_eq!(
            refusal,
            format!(
                "Invalid parameters: {tool}: start (2026-09-01 00:00:00.250 UTC) is after end \
                 (2026-08-31 00:00:00 UTC)"
            ),
            "{tool} names both instants of the refused window in UTC"
        );
    }
}

/// The same read as an MCP client receives it, through `POST /mcp`. WHOOP's
/// terms keep its night off that wire, so the client gets the window, the
/// last sync and a count of what it was not given: no session, no instant
/// of one.
#[tokio::test]
async fn a_stored_night_over_mcp_answers_the_window_and_the_sync_and_withholds_the_night() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = athlete_with_a_stored_day(&resources, "golden-mcp-sleep").await;

    let envelope = mcp_call(&resources, &athlete, "get_sleep_sessions", august()).await;
    let payload = mcp_payload(&envelope);

    assert_eq!(
        dates_in(&payload),
        table(&[
            ("/last_synced_at/whoop", "2026-08-28T11:00:00.250125+00:00"),
            ("/range/end", "2026-08-31T00:00:00+00:00"),
            ("/range/start", "2026-08-01T00:00:00+00:00"),
        ]),
        "{envelope}"
    );
    assert_eq!(
        payload["_unavailable_over_this_interface"]["items_unavailable"], 1,
        "{envelope}"
    );
}
